use std::sync::OnceLock;

pub const NEW_LAYOUT_BUILD: u32 = 22000;

#[derive(Debug, PartialEq, Eq)]
pub enum MonitorFlow {
    AddMonitorClient,
    AddProcessMonitor,
}

#[derive(Debug)]
pub struct Profile {
    pub os_build: u32,
    pub extension_version: &'static str,
    pub activities: bool,
    pub flow: MonitorFlow,
    pub query_shift: u32,
    pub monitor_shift: u32,
    pub detach_shift: u32,
    pub continue_shift: u32,
    pub add_client_shift: u32,
    pub pdem_shift: u32,
    pub split_line: u32,
    pub open_line: u32,
    pub open_expr: &'static [u8],
    pub open_retry: bool,
}

pub static P22000: Profile = Profile {
    os_build: 22000,
    extension_version: "220604",
    activities: true,
    flow: MonitorFlow::AddMonitorClient,
    query_shift: 0,
    monitor_shift: 0,
    detach_shift: 0,
    continue_shift: 0,
    add_client_shift: 0,
    pdem_shift: 0,
    split_line: 0xb2,
    open_line: 0x105,
    open_expr: b"hr\0",
    open_retry: true,
};

pub static P19041: Profile = Profile {
    os_build: 19041,
    extension_version: "200804",
    activities: false,
    flow: MonitorFlow::AddProcessMonitor,
    query_shift: 1,
    monitor_shift: 2,
    detach_shift: 3,
    continue_shift: 4,
    add_client_shift: 10,
    pdem_shift: 11,
    split_line: 0xa4,
    open_line: 0xc6,
    open_expr: b"XbtpOpenSession2( consoleIpAddress, explicitPort, &auxInfo, extensionId, iid, ppvObject)\0",
    open_retry: false,
};

impl Profile {
    pub fn query(&self, line: u32) -> u32 {
        line - self.query_shift
    }
    pub fn monitor(&self, line: u32) -> u32 {
        line - self.monitor_shift
    }
    pub fn detach(&self, line: u32) -> u32 {
        line - self.detach_shift
    }
    pub fn cont(&self, line: u32) -> u32 {
        line - self.continue_shift
    }
    pub fn add_client(&self, line: u32) -> u32 {
        line - self.add_client_shift
    }
    pub fn pdem(&self, line: u32) -> u32 {
        line - self.pdem_shift
    }
}

pub fn for_build(os_build: u32) -> &'static Profile {
    if os_build < NEW_LAYOUT_BUILD {
        &P19041
    } else {
        &P22000
    }
}

#[cfg(test)]
thread_local! {
    pub static TEST_PROFILE: core::cell::Cell<Option<&'static Profile>> = const { core::cell::Cell::new(None) };
}

#[cfg(test)]
pub struct UseProfile(Option<&'static Profile>);

#[cfg(test)]
impl UseProfile {
    pub fn on(p: &'static Profile) -> UseProfile {
        UseProfile(TEST_PROFILE.with(|t| t.replace(Some(p))))
    }
}

#[cfg(test)]
impl Drop for UseProfile {
    fn drop(&mut self) {
        TEST_PROFILE.with(|t| t.set(self.0));
    }
}

pub fn emulated() -> &'static Profile {
    static CURRENT: OnceLock<&'static Profile> = OnceLock::new();
    CURRENT.get_or_init(|| for_build(compat_common::os_host::k32legacy::console_os_build()))
}

#[cfg(not(test))]
pub fn current() -> &'static Profile {
    emulated()
}

#[cfg(test)]
pub fn current() -> &'static Profile {
    TEST_PROFILE.with(|t| t.get()).unwrap_or(&P22000)
}
