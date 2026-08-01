local outputPath = [[D:\work\ideas\ScrapMap\artifacts\ce-position-capture.txt]]
local watchedAddress = 0x000002274dc6a740
local output = io.open(outputPath, "w")

local function log(line)
  if output then
    output:write(line .. "\n")
    output:flush()
  end
end

local function finish()
  pcall(function() debug_removeBreakpoint(watchedAddress) end)
  if output then output:close(); output = nil end
  closeCE()
end

if not openProcess("ScrapMechanic.exe") then
  log("ERROR openProcess")
  finish()
  return
end

log(string.format("pid=%d module=%X watched=%X", getOpenedProcessID(), getAddress("ScrapMechanic.exe"), watchedAddress))

if not debugProcess(1) then
  log("ERROR debugProcess")
  finish()
  return
end

local hits = 0
local ok = debug_setBreakpoint(watchedAddress, 4, bptAccess, bpmDebugRegister, function()
  local context = debug_getCurrentContextTable()
  hits = hits + 1
  if context then
    log(string.format(
      "hit=%d rip=%X rax=%X rbx=%X rcx=%X rdx=%X rsi=%X rdi=%X rbp=%X rsp=%X r8=%X r9=%X r10=%X r11=%X r12=%X r13=%X r14=%X r15=%X ins=%s",
      hits, context.RIP or 0, context.RAX or 0, context.RBX or 0, context.RCX or 0,
      context.RDX or 0, context.RSI or 0, context.RDI or 0, context.RBP or 0,
      context.RSP or 0, context.R8 or 0, context.R9 or 0, context.R10 or 0,
      context.R11 or 0, context.R12 or 0, context.R13 or 0, context.R14 or 0,
      context.R15 or 0, disassemble(context.RIP or 0)
    ))
  else
    log("hit=" .. hits .. " no-context")
  end
  if hits >= 24 then
    createTimer(100, finish)
  end
  return 1
end)

if not ok then
  log("ERROR breakpoint")
  finish()
  return
end

log("armed")
createTimer(15000, finish)
