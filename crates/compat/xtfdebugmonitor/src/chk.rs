use xtfmsg::sys::HRESULT;

fn z(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

#[cfg(test)]
thread_local! {
    pub static RECORDED: core::cell::RefCell<Vec<(String, u32, String, HRESULT)>> = const { core::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub fn take_recorded() -> Vec<(String, u32, String, HRESULT)> {
    RECORDED.with(|r| core::mem::take(&mut *r.borrow_mut()))
}

pub fn chk(file: &str, func: &str, line: u32, name: &str, hr: HRESULT, msg: &str) {
    #[cfg(test)]
    RECORDED.with(|r| r.borrow_mut().push((func.to_string(), line, name.to_string(), hr)));
    let short = z(&format!("XtfCHK: {func}: {line}"));
    let long = z(&format!("XtfCHK: {file}: {func}: {line}"));
    xtfmsg::chk::chk(&short, &long, &z(name), hr, &z(msg));
}
