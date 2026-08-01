//! Development-only read-only position candidate scanner.
//!
//! Usage: native_position_probe <pid> <cell-x> <cell-y> [delay-seconds]

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("Windows only");
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), String> {
    use std::{env, thread, time::Duration};

    let mut args = env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("dump") {
        let pid = parse::<u32>(args.next(), "pid")?;
        let address_text = args.next().ok_or_else(|| "missing address".to_owned())?;
        let address = usize::from_str_radix(address_text.trim_start_matches("0x"), 16)
            .map_err(|_| "invalid address".to_owned())?;
        let reader = Reader::open(pid)?;
        let start = address.saturating_sub(128);
        let mut bytes = [0_u8; 256];
        reader
            .read(start, &mut bytes)
            .map_err(|_| "read failed".to_owned())?;
        for (row, chunk) in bytes.chunks(16).enumerate() {
            print!("0x{:016x} ", start + row * 16);
            for byte in chunk {
                print!("{byte:02x} ");
            }
            println!();
        }
        return Ok(());
    }
    if first.as_deref() == Some("chain") {
        let pid = parse::<u32>(args.next(), "pid")?;
        let address_text = args.next().ok_or_else(|| "missing address".to_owned())?;
        let address = usize::from_str_radix(address_text.trim_start_matches("0x"), 16)
            .map_err(|_| "invalid address".to_owned())?;
        let reader = Reader::open(pid)?;
        reader.find_pointer_chains(address, 4)?;
        return Ok(());
    }
    if first.as_deref() == Some("watch") {
        let pid = parse::<u32>(args.next(), "pid")?;
        let address_text = args.next().ok_or_else(|| "missing address".to_owned())?;
        let address = usize::from_str_radix(address_text.trim_start_matches("0x"), 16)
            .map_err(|_| "invalid address".to_owned())?;
        let seconds = args
            .next()
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| "invalid seconds".to_owned())
            })
            .transpose()?
            .unwrap_or(20);
        let reader = Reader::open(pid)?;
        let mut previous = None;
        for _ in 0..seconds * 10 {
            let xyz = reader.read_xyz_f32(address)?;
            if previous != Some(xyz) {
                println!("{:.5},{:.5},{:.5}", xyz.0, xyz.1, xyz.2);
                previous = Some(xyz);
            }
            thread::sleep(Duration::from_millis(100));
        }
        return Ok(());
    }
    let pid = parse::<u32>(first, "pid")?;
    let cell_x = parse::<i32>(args.next(), "cell-x")?;
    let cell_y = parse::<i32>(args.next(), "cell-y")?;
    let delay = args
        .next()
        .map(|value| value.parse::<u64>().map_err(|_| "invalid delay".to_owned()))
        .transpose()?
        .unwrap_or(10);
    if args.next().is_some() {
        return Err("too many arguments".to_owned());
    }

    let reader = Reader::open(pid)?;
    let bounds = Bounds::for_cell(cell_x, cell_y);
    let candidates = reader.scan(bounds)?;
    println!(
        "BASELINE {} candidates; keep the player still",
        candidates.len()
    );
    thread::sleep(Duration::from_secs(5));
    let first = candidates
        .into_iter()
        .filter_map(|candidate| reader.sample_stable(candidate, bounds))
        .collect::<Vec<_>>();
    println!("STABLE {} candidates; move the player now", first.len());
    thread::sleep(Duration::from_secs(delay));
    let mut changed = first
        .into_iter()
        .filter_map(|candidate| reader.second_change(candidate, bounds))
        .collect::<Vec<_>>();
    changed.sort_by(|left, right| {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    println!("CHANGED {} candidates", changed.len());
    for item in changed.into_iter().take(200) {
        println!(
            "{} 0x{:016x} ({:.4},{:.4},{:.4}) -> ({:.4},{:.4},{:.4}) -> ({:.4},{:.4},{:.4}) d1={:.4} d2={:.4}",
            item.kind,
            item.address,
            item.before_x,
            item.before_y,
            item.before_z,
            item.middle_x,
            item.middle_y,
            item.middle_z,
            item.after_x,
            item.after_y,
            item.after_z,
            item.distance_a,
            item.distance_b
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn parse<T: std::str::FromStr>(value: Option<String>, name: &str) -> Result<T, String> {
    value
        .ok_or_else(|| format!("missing {name}"))?
        .parse()
        .map_err(|_| format!("invalid {name}"))
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy)]
struct Bounds {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

#[cfg(target_os = "windows")]
impl Bounds {
    fn for_cell(x: i32, y: i32) -> Self {
        const CELL: f64 = 64.0;
        const MARGIN: f64 = 2.0;
        Self {
            min_x: x as f64 * CELL - MARGIN,
            max_x: (x + 1) as f64 * CELL + MARGIN,
            min_y: y as f64 * CELL - MARGIN,
            max_y: (y + 1) as f64 * CELL + MARGIN,
        }
    }

    fn contains(self, x: f64, y: f64) -> bool {
        x.is_finite()
            && y.is_finite()
            && x >= self.min_x
            && x <= self.max_x
            && y >= self.min_y
            && y <= self.max_y
    }

    fn contains_xyz(self, x: f64, y: f64, z: f64) -> bool {
        self.contains(x, y) && z.is_finite() && (-100.0..=500.0).contains(&z)
    }
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy)]
enum CandidateKind {
    F32,
    F64,
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy)]
struct Candidate {
    address: usize,
    kind: CandidateKind,
    x: f64,
    y: f64,
    z: f64,
}

#[cfg(target_os = "windows")]
struct FirstChange {
    candidate: Candidate,
    x: f64,
    y: f64,
    z: f64,
    distance: f64,
}

#[cfg(target_os = "windows")]
struct Change {
    address: usize,
    kind: &'static str,
    before_x: f64,
    before_y: f64,
    before_z: f64,
    middle_x: f64,
    middle_y: f64,
    middle_z: f64,
    after_x: f64,
    after_y: f64,
    after_z: f64,
    distance_a: f64,
    distance_b: f64,
    score: f64,
}

#[cfg(target_os = "windows")]
struct Reader(windows::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
#[derive(Clone)]
struct PointerPath {
    target: usize,
    chain: String,
}

#[cfg(target_os = "windows")]
impl Reader {
    fn open(pid: u32) -> Result<Self, String> {
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
        };
        if pid == 0 {
            return Err("pid must be non-zero".to_owned());
        }
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                false,
                pid,
            )
        }
        .map(Self)
        .map_err(|error| format!("OpenProcess failed: {error}"))
    }

    fn scan(&self, bounds: Bounds) -> Result<Vec<Candidate>, String> {
        use std::{ffi::c_void, mem::size_of};
        use windows::Win32::System::Memory::{
            VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE, PAGE_GUARD,
            PAGE_NOACCESS, PAGE_READWRITE, PAGE_WRITECOPY,
        };

        const CHUNK: usize = 1024 * 1024;
        const MAX_CANDIDATES: usize = 2_000_000;
        let mut address = 0_usize;
        let mut candidates = Vec::new();
        loop {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            let queried = unsafe {
                VirtualQueryEx(
                    self.0,
                    Some(address as *const c_void),
                    &mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if queried == 0 {
                break;
            }
            let base = info.BaseAddress as usize;
            let next = base.saturating_add(info.RegionSize);
            if next <= address {
                break;
            }
            if info.State == MEM_COMMIT
                && info.Type == MEM_PRIVATE
                && !info.Protect.contains(PAGE_GUARD)
                && !info.Protect.contains(PAGE_NOACCESS)
                && (info.Protect.contains(PAGE_READWRITE) || info.Protect.contains(PAGE_WRITECOPY))
            {
                let mut offset = 0_usize;
                while offset < info.RegionSize {
                    let length = CHUNK.min(info.RegionSize - offset);
                    let mut bytes = vec![0_u8; length];
                    if self.read(base + offset, &mut bytes).is_ok() {
                        scan_chunk(base + offset, &bytes, bounds, &mut candidates);
                        if candidates.len() > MAX_CANDIDATES {
                            return Err("candidate safety limit exceeded".to_owned());
                        }
                    }
                    offset += length;
                }
            }
            address = next;
        }
        Ok(candidates)
    }

    fn read_xyz_f32(&self, address: usize) -> Result<(f32, f32, f32), String> {
        let mut bytes = [0_u8; 12];
        self.read(address, &mut bytes)
            .map_err(|_| "read failed".to_owned())?;
        Ok((
            f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        ))
    }

    fn find_pointer_chains(&self, target: usize, depth: usize) -> Result<(), String> {
        const MAX_FRONTIER: usize = 2_000;
        let (module_base, module_size) = main_module(self.0)?;
        let module_end = module_base + module_size;
        let mut frontier = vec![PointerPath {
            target,
            chain: format!("XYZ@0x{target:x}"),
        }];
        for level in 1..=depth {
            let references = self.scan_pointer_references(&frontier)?;
            println!("LEVEL {level}: {} references", references.len());
            if level == 1 {
                for (source, path) in references.iter().take(200) {
                    println!("REF 0x{source:016x} {}", path.chain);
                }
            }
            let mut next = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for (source, path) in references {
                if (module_base..module_end).contains(&source) {
                    println!("MODULE +0x{:x} -> {}", source - module_base, path.chain);
                } else if seen.insert(source) && next.len() < MAX_FRONTIER {
                    next.push(PointerPath {
                        target: source,
                        chain: path.chain,
                    });
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(())
    }

    fn scan_pointer_references(
        &self,
        targets: &[PointerPath],
    ) -> Result<Vec<(usize, PointerPath)>, String> {
        use std::{ffi::c_void, mem::size_of};
        use windows::Win32::System::Memory::{
            VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
        };

        const CHUNK: usize = 1024 * 1024;
        const MAX_RESULTS: usize = 20_000;
        const MAX_FIELD_OFFSET: usize = 0x400;
        let mut sorted = targets.to_vec();
        sorted.sort_by_key(|item| item.target);
        let mut address = 0_usize;
        let mut output = Vec::new();
        loop {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            let queried = unsafe {
                VirtualQueryEx(
                    self.0,
                    Some(address as *const c_void),
                    &mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if queried == 0 {
                break;
            }
            let base = info.BaseAddress as usize;
            let next = base.saturating_add(info.RegionSize);
            if next <= address {
                break;
            }
            if info.State == MEM_COMMIT
                && !info.Protect.contains(PAGE_GUARD)
                && !info.Protect.contains(PAGE_NOACCESS)
            {
                let mut offset = 0_usize;
                while offset < info.RegionSize {
                    let length = CHUNK.min(info.RegionSize - offset);
                    let mut bytes = vec![0_u8; length];
                    if self.read(base + offset, &mut bytes).is_ok() {
                        for index in (0..bytes.len().saturating_sub(8)).step_by(8) {
                            let value =
                                u64::from_le_bytes(bytes[index..index + 8].try_into().unwrap())
                                    as usize;
                            if value < 0x10000 {
                                continue;
                            }
                            let first = sorted.partition_point(|item| item.target < value);
                            for target in sorted[first..]
                                .iter()
                                .take_while(|item| item.target - value <= MAX_FIELD_OFFSET)
                            {
                                let field_offset = target.target - value;
                                output.push((
                                    base + offset + index,
                                    PointerPath {
                                        target: base + offset + index,
                                        chain: format!(
                                            "[ptr]+0x{field_offset:x} -> {}",
                                            target.chain
                                        ),
                                    },
                                ));
                                if output.len() >= MAX_RESULTS {
                                    return Ok(output);
                                }
                            }
                        }
                    }
                    offset += length;
                }
            }
            address = next;
        }
        Ok(output)
    }

    fn sample(&self, candidate: Candidate) -> Option<(f64, f64, f64)> {
        match candidate.kind {
            CandidateKind::F32 => {
                let mut bytes = [0_u8; 12];
                self.read(candidate.address, &mut bytes).ok()?;
                Some((
                    f32::from_le_bytes(bytes[0..4].try_into().ok()?) as f64,
                    f32::from_le_bytes(bytes[4..8].try_into().ok()?) as f64,
                    f32::from_le_bytes(bytes[8..12].try_into().ok()?) as f64,
                ))
            }
            CandidateKind::F64 => {
                let mut bytes = [0_u8; 24];
                self.read(candidate.address, &mut bytes).ok()?;
                Some((
                    f64::from_le_bytes(bytes[0..8].try_into().ok()?),
                    f64::from_le_bytes(bytes[8..16].try_into().ok()?),
                    f64::from_le_bytes(bytes[16..24].try_into().ok()?),
                ))
            }
        }
    }

    fn sample_stable(&self, candidate: Candidate, bounds: Bounds) -> Option<FirstChange> {
        let (x, y, z) = self.sample(candidate)?;
        if !bounds.contains_xyz(x, y, z) {
            return None;
        }
        let distance = (x - candidate.x).hypot(y - candidate.y);
        if distance > 0.002 || (z - candidate.z).abs() > 0.002 {
            return None;
        }
        Some(FirstChange {
            candidate,
            x,
            y,
            z,
            distance,
        })
    }

    fn second_change(&self, first: FirstChange, bounds: Bounds) -> Option<Change> {
        let (after_x, after_y, after_z) = self.sample(first.candidate)?;
        if !bounds.contains_xyz(after_x, after_y, after_z) || (after_z - first.z).abs() > 5.0 {
            return None;
        }
        let distance_b = (after_x - first.x).hypot(after_y - first.y);
        if !(0.1..=20.0).contains(&distance_b) {
            return None;
        }
        let score = (first.distance - distance_b).abs() + (after_z - first.z).abs();
        Some(Change {
            address: first.candidate.address,
            kind: match first.candidate.kind {
                CandidateKind::F32 => "f32",
                CandidateKind::F64 => "f64",
            },
            before_x: first.candidate.x,
            before_y: first.candidate.y,
            before_z: first.candidate.z,
            middle_x: first.x,
            middle_y: first.y,
            middle_z: first.z,
            after_x,
            after_y,
            after_z,
            distance_a: first.distance,
            distance_b,
            score,
        })
    }

    fn read(&self, address: usize, output: &mut [u8]) -> Result<(), ()> {
        use std::ffi::c_void;
        use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
        let mut read = 0_usize;
        unsafe {
            ReadProcessMemory(
                self.0,
                address as *const c_void,
                output.as_mut_ptr().cast(),
                output.len(),
                Some(&mut read),
            )
        }
        .map_err(|_| ())?;
        (read == output.len()).then_some(()).ok_or(())
    }
}

#[cfg(target_os = "windows")]
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
fn main_module(process: windows::Win32::Foundation::HANDLE) -> Result<(usize, usize), String> {
    use std::mem::size_of;
    use windows::Win32::System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Module32FirstW, MODULEENTRY32W, TH32CS_SNAPMODULE,
            TH32CS_SNAPMODULE32,
        },
        Threading::GetProcessId,
    };
    let pid = unsafe { GetProcessId(process) };
    if pid == 0 {
        return Err("GetProcessId failed".to_owned());
    }
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) }
            .map_err(|error| format!("module snapshot failed: {error}"))?;
    let mut entry = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let result = unsafe { Module32FirstW(snapshot, &mut entry) };
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
    }
    result.map_err(|error| format!("main module failed: {error}"))?;
    Ok((entry.modBaseAddr as usize, entry.modBaseSize as usize))
}

#[cfg(target_os = "windows")]
fn scan_chunk(base: usize, bytes: &[u8], bounds: Bounds, output: &mut Vec<Candidate>) {
    for offset in (0..bytes.len().saturating_sub(24)).step_by(4) {
        let x = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as f64;
        let y = f32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as f64;
        let z = f32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as f64;
        if bounds.contains_xyz(x, y, z) {
            output.push(Candidate {
                address: base + offset,
                kind: CandidateKind::F32,
                x,
                y,
                z,
            });
        }
        if offset % 8 == 0 {
            let x = f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
            let y = f64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
            let z = f64::from_le_bytes(bytes[offset + 16..offset + 24].try_into().unwrap());
            if bounds.contains_xyz(x, y, z) {
                output.push(Candidate {
                    address: base + offset,
                    kind: CandidateKind::F64,
                    x,
                    y,
                    z,
                });
            }
        }
    }
}
