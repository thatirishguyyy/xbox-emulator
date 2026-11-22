pub const FIRST_22000_BUILD: u32 = 22000;
pub const RUN_HANDLER_LINE_SHIFT_19041: u32 = 52;
pub const CREATE_PROCESS_LINE_SHIFT_19041: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConsoleCopy {
    Build19041,
    Build22000,
}

pub fn os_build() -> u32 {
    compat_common::os_host::routes::current_build()
}

pub fn for_build(build: u32) -> ConsoleCopy {
    if build < FIRST_22000_BUILD {
        ConsoleCopy::Build19041
    } else {
        ConsoleCopy::Build22000
    }
}

pub fn current() -> ConsoleCopy {
    for_build(os_build())
}

impl ConsoleCopy {
    pub fn run_handler_line(self, line_22000: u32) -> u32 {
        match self {
            ConsoleCopy::Build19041 => line_22000 - RUN_HANDLER_LINE_SHIFT_19041,
            ConsoleCopy::Build22000 => line_22000,
        }
    }

    pub fn create_process_line(self, line_22000: u32) -> u32 {
        match self {
            ConsoleCopy::Build19041 => line_22000 - CREATE_PROCESS_LINE_SHIFT_19041,
            ConsoleCopy::Build22000 => line_22000,
        }
    }

    pub fn has_host_jobs(self) -> bool {
        self == ConsoleCopy::Build22000
    }

    pub fn has_client2(self) -> bool {
        self == ConsoleCopy::Build22000
    }

    pub fn has_run_handler_activity(self) -> bool {
        self == ConsoleCopy::Build22000
    }
}
