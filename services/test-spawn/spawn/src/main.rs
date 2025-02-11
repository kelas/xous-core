#![no_std]
#![no_main]

enum StartupCommand {
    Unhandled = 0,
    WriteMemory = 1,
    WriteArgs = 2,
    WriteEnvironment = 3,
    PingResponse = 4,
    FinishStartup = 255,
}

impl From<xous::MessageId> for StartupCommand {
    fn from(src: xous::MessageId) -> StartupCommand {
        match src {
            1 => StartupCommand::WriteMemory,
            2 => StartupCommand::WriteArgs,
            3 => StartupCommand::WriteEnvironment,
            4 => StartupCommand::PingResponse,
            255 => StartupCommand::FinishStartup,
            _ => StartupCommand::Unhandled,
        }
    }
}

#[panic_handler]
fn handle_panic(_arg: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn init(server1: u32, server2: u32, server3: u32, server4: u32) -> ! {
    let server = xous::SID::from_u32(server1, server2, server3, server4);
    loop {
        if let Ok(xous::Result::Message(envelope)) =
            xous::rsyscall(xous::SysCall::ReceiveMessage(server))
        {
            match envelope.id().into() {
                StartupCommand::WriteMemory => write_memory(envelope.body.memory_message()),
                StartupCommand::FinishStartup => finish_startup(server, envelope),
                StartupCommand::PingResponse => ping_response(envelope),

                _ => panic!("Unsupported"),
            }
        }
    }
}

fn ping_response(envelope: xous::MessageEnvelope) {
    if let Some(msg) = envelope.body.scalar_message() {
        if envelope.body.is_blocking() {
            xous::syscall::return_scalar(envelope.sender, msg.arg1 + 1).unwrap();
        }
    }
}

fn write_memory(memory: Option<&xous::MemoryMessage>) {
    let memory = match memory {
        Some(s) => s,
        None => return,
    };

    let mut target_memory = xous::map_memory(
        None,
        memory.offset,
        memory.buf.len(),
        xous::MemoryFlags::R | xous::MemoryFlags::W,
    )
    .unwrap();

    for (src, dest) in memory
        .buf
        .as_slice::<usize>()
        .iter()
        .zip(target_memory.as_slice_mut())
    {
        *dest = *src;
    }
}

fn finish_startup(server: xous::SID, envelope: xous::MessageEnvelope) -> ! {
    let entrypoint = envelope.body.scalar_message().unwrap().arg1;
    drop(envelope);
    xous::destroy_server(server).unwrap();
    let entry_fn = unsafe { core::mem::transmute::<_, fn() -> !>(entrypoint) };
    entry_fn();
}
