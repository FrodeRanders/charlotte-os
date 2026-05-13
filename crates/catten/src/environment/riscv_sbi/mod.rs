pub struct SbiReturn {
    pub error: u64,
    pub value: u64,
}

#[repr(i32)]
pub enum SbiExtensionId {
    Base = 0x10,
    Timer = b"TIME",                   // "TIME"
    Ipi = b"sIP",                      // "sIP"
    RFence = b"RFNC",                  // "RFNC"
    HartStateMgmt = b"HSM",            // "HSM"
    SystemReset = b"SRST",             // "SRST"
    PerformanceMonitoring = b"PMO",    // "PMO"
    DebugConsole = b"DBNC",            // "DBNC"
    SystemSuspend = b"SUSP",           // "SUSP"
    Cppc = b"CPPC",                    // "CPPC"
    NestedAcceleration = b"NACL",      // "NACL"
    StealTimeAccounting = b"STA",      // "STA"
    SupervisorSoftwareEvents = b"SSE", // "SSE"
    SbiFirmwareFeatures = b"FWFT",     // "FWFT"
    DebugTriggers = b"DBTR",           // "DBTR"
    MessageProxy = b"MPXY",            // "MPXY"
}

#[repr(i32)]
pub enum SbiBaseFunctionId {
    GetSbiSpecVersion = 0,
    GetSbiImplId = 1,
    GetSbiImplVersion = 2,
    ProbeExtension = 3,
    GetMachineVendorId = 4,
    GetMachineArchId = 5,
    GetMachineImplId = 6,
}

#[macro_export]
macro_rules! call_sbi {
    ($extension:expr, $function:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr) => {{
        let mut return_value = SbiReturn {
            error: 0,
            value: 0,
        };
        unsafe {
            core::arch::asm!(
                "ecall",
                earlyin("a7") $extension,
                earlyin("a6") $function,
                in("a0") $a0,
                in("a1") $a1,
                in("a2") $a2,
                in("a3") $a3,
                in("a4") $a4,
                in("a5") $a5,
                lateout("a0") return_value.error,
                lateout("a1") return_value.value,
            );
        }
        return_value
    }};
}
pub use call_sbi;
