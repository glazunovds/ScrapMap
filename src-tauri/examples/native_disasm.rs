//! Development-only PE RVA disassembler.
//! Usage: native_disasm <exe> <rva-hex> [bytes]

use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter};
use std::{env, fs};

fn u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    data.get(offset..offset + 2)
        .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
        .ok_or("truncated PE".into())
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    data.get(offset..offset + 4)
        .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
        .ok_or("truncated PE".into())
}

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let path = args.next().ok_or("missing exe")?;
    let rva = u32::from_str_radix(
        args.next().ok_or("missing rva")?.trim_start_matches("0x"),
        16,
    )
    .map_err(|_| "invalid rva")?;
    let wanted = args
        .next()
        .map(|v| v.parse::<usize>())
        .transpose()
        .map_err(|_| "invalid byte count")?
        .unwrap_or(256);
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let pe = u32_at(&data, 0x3c)? as usize;
    let section_count = u16_at(&data, pe + 6)? as usize;
    let optional_size = u16_at(&data, pe + 20)? as usize;
    let sections = pe + 24 + optional_size;
    let mut file_offset = None;
    for index in 0..section_count {
        let section = sections + index * 40;
        let virtual_size = u32_at(&data, section + 8)?;
        let virtual_address = u32_at(&data, section + 12)?;
        let raw_size = u32_at(&data, section + 16)?;
        let raw_offset = u32_at(&data, section + 20)?;
        let span = virtual_size.max(raw_size);
        if rva >= virtual_address && rva < virtual_address + span {
            file_offset = Some((raw_offset + rva - virtual_address) as usize);
            break;
        }
    }
    let offset = file_offset.ok_or("RVA outside sections")?;
    let end = (offset + wanted).min(data.len());
    let mut decoder = Decoder::with_ip(64, &data[offset..end], rva as u64, DecoderOptions::NONE);
    let mut formatter = IntelFormatter::new();
    let mut rendered = String::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        rendered.clear();
        formatter.format(&instruction, &mut rendered);
        let start = offset + (instruction.ip() - rva as u64) as usize;
        let bytes = &data[start..start + instruction.len()];
        println!(
            "{:08X} {:<28} {}",
            instruction.ip(),
            bytes.iter().map(|b| format!("{b:02X}")).collect::<String>(),
            rendered
        );
    }
    Ok(())
}
