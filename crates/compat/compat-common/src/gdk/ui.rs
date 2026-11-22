use super::win::{self, Dialog, Edit, List};
use super::xasync::{self, Cancel, Payload};
use super::{cstr, log, Guid, Iface, This, E_ABORT, E_INVALIDARG, S_OK};
use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const CLSID: Guid = Guid::parse("dfcd4649-4ff8-4043-ba07-35d607df98b0");
pub const IID_V1: Guid = Guid::parse("6eeaa73e-9669-43ad-a2c7-d0da4e1f50a1");
pub const IID_V2: Guid = Guid::parse("72c4e70f-4a67-481b-bb84-aa72eda14fe5");
pub const IID_FC0B: Guid = Guid::parse("fc0b9bd0-8898-4643-bee0-4ee6ee034cb6");

static OP_MESSAGE: &[u8] = b"XGameUiShowMessageDialogAsync\0";
static OP_STATUS: &[u8] = b"XGameUiShowAsync\0";
static OP_TEXT: &[u8] = b"XGameUiShowTextEntryAsync\0";
static OP_WEB: &[u8] = b"XGameUiShowWebAuthenticationAsync\0";
static OP_PICKER: &[u8] = b"XGameUiShowPlayerPickerAsync\0";

pub const E_ERROR_DIALOG_CANCELLED: i32 = E_ABORT;

fn show(d: Dialog, c: &Arc<Cancel>) -> win::Outcome {
    win::show(d, Some(c.window.clone()))
}

pub fn message_dialog(title: String, content: String, buttons: Vec<String>, default: u32, cancel: u32) -> impl FnOnce(Arc<Cancel>) -> Payload + Send {
    move |c| {
        let n = buttons.len();
        let d = Dialog {
            title,
            text: content,
            buttons,
            default: (default as usize).min(n.saturating_sub(1)),
            cancel: Some((cancel as usize).min(n.saturating_sub(1))),
            ..Default::default()
        };
        let o = show(d, &c);
        if c.is_set() {
            return Payload::status(E_ABORT);
        }
        let b = o.button.unwrap_or(cancel as usize) as u32;
        Payload::value(b)
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn message_async(
    _t: This, block: *mut c_void, title: *const u8, content: *const u8, b1: *const u8, b2: *const u8, b3: *const u8, default: u32, cancel: u32,
) -> i32 {
    let buttons: Vec<String> = [b1, b2, b3].iter().filter_map(|b| cstr(*b)).filter(|s| !s.is_empty()).collect();
    if buttons.is_empty() {
        return E_INVALIDARG;
    }
    let (t, m) = (cstr(title).unwrap_or_default(), cstr(content).unwrap_or_default());
    log("XGameUiShowMessageDialogAsync", &format!("'{t}' {} buttons", buttons.len()));
    xasync::run_with(block, OP_MESSAGE, Box::new(message_dialog(t, m, buttons, default, cancel)))
}

unsafe extern "system" fn message_result(_t: This, block: *mut c_void, out: *mut u32) -> i32 {
    xasync::value(block, OP_MESSAGE, out)
}

fn error_text(hr: i32) -> String {
    extern "system" {
        fn FormatMessageW(flags: u32, src: *const c_void, id: u32, lang: u32, buf: *mut u16, n: u32, args: *const c_void) -> u32;
    }
    let mut buf = [0u16; 512];
    let n = unsafe { FormatMessageW(0x1000 | 0x200, core::ptr::null(), hr as u32, 0, buf.as_mut_ptr(), buf.len() as u32, core::ptr::null()) } as usize;
    let s = String::from_utf16_lossy(&buf[..n.min(buf.len())]).trim().to_string();
    if s.is_empty() { format!("An error occurred (0x{:08X}).", hr as u32) } else { format!("{s}\n\nError code 0x{:08X}", hr as u32) }
}

unsafe extern "system" fn error_async(_t: This, block: *mut c_void, error: i32, _ctx: i32) -> i32 {
    let text = error_text(error);
    xasync::run_with(block, OP_STATUS, Box::new(move |c| {
        show(Dialog { title: "Error".into(), text, buttons: vec!["OK".into()], cancel: Some(0), ..Default::default() }, &c);
        Payload::status(if c.is_set() { E_ABORT } else { S_OK })
    }))
}

fn gamertag_for(xuid: u64) -> String {
    let u = super::user::user();
    if xuid == u.xuid { u.gamertag.clone() } else { super::social::name_for(xuid) }
}

unsafe extern "system" fn profile_async(_t: This, block: *mut c_void, _u: usize, target: u64) -> i32 {
    let text = format!("Gamertag: {}\nXUID: {target}\n{}", gamertag_for(target), super::social::relationship_text(target));
    xasync::run_with(block, OP_STATUS, Box::new(move |c| {
        show(Dialog { title: "Profile".into(), text, buttons: vec!["Close".into()], cancel: Some(0), ..Default::default() }, &c);
        Payload::status(if c.is_set() { E_ABORT } else { S_OK })
    }))
}

unsafe extern "system" fn achievements_async(_t: This, block: *mut c_void, u: usize, title_id: u32) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let text = super::social::achievements_text(title_id);
    xasync::run_with(block, OP_STATUS, Box::new(move |c| {
        show(Dialog { title: "Achievements".into(), text, buttons: vec!["Close".into()], cancel: Some(0), ..Default::default() }, &c);
        Payload::status(if c.is_set() { E_ABORT } else { S_OK })
    }))
}

unsafe extern "system" fn status_result(_t: This, block: *mut c_void) -> i32 {
    xasync::status(block)
}

pub fn invite_flow(prompt: String, session: String, text: String, ctx: String) -> impl FnOnce(Arc<Cancel>) -> Payload + Send {
    move |c| {
        let friends = super::social::friends();
        if friends.is_empty() {
            show(Dialog { title: "Invite".into(), text: "There's no one on your friends list to invite.".into(), buttons: vec!["OK".into()], cancel: Some(0), ..Default::default() }, &c);
            return Payload::status(E_ABORT);
        }
        let items: Vec<String> = friends.iter().map(|f| f.1.clone()).collect();
        let d = Dialog {
            title: "Invite to game".into(),
            text: prompt,
            list: Some(List { items, multi: true, preselected: Vec::new(), min: 1, max: 100 }),
            buttons: vec!["Send invites".into(), "Cancel".into()],
            default: 0,
            cancel: Some(1),
            ..Default::default()
        };
        let o = show(d, &c);
        if c.is_set() || o.button != Some(0) || o.selected.is_empty() {
            return Payload::status(E_ABORT);
        }
        let to: Vec<u64> = o.selected.iter().filter_map(|&i| friends.get(i).map(|f| f.0)).collect();
        Payload::status(super::social::send_invites(&to, &session, &text, &ctx))
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn invite_async(_t: This, block: *mut c_void, u: usize, scid: *const u8, tmpl: *const u8, sid: *const u8, text: *const u8, ctx: *const u8) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let session = format!("{}/{}/{}", cstr(scid).unwrap_or_default(), cstr(tmpl).unwrap_or_default(), cstr(sid).unwrap_or_default());
    let prompt = "Choose friends to invite.".to_string();
    xasync::run_with(block, OP_STATUS, Box::new(invite_flow(prompt, session, cstr(text).unwrap_or_default(), cstr(ctx).unwrap_or_default())))
}

unsafe extern "system" fn mpa_invite_async(_t: This, block: *mut c_void, u: usize) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let session = super::social::activity_session();
    xasync::run_with(block, OP_STATUS, Box::new(invite_flow("Choose friends to invite to your activity.".into(), session, String::new(), String::new())))
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn picker_async(
    _t: This, block: *mut c_void, u: usize, prompt: *const u8, from_n: u32, from: *const u64, pre_n: u32, pre: *const u64, min: u32, max: u32,
) -> i32 {
    if u == 0 || (from_n > 0 && from.is_null()) || (pre_n > 0 && pre.is_null()) || min > max || max == 0 {
        return E_INVALIDARG;
    }
    let players: Vec<u64> = (0..from_n as usize).map(|i| *from.add(i)).collect();
    let presel: Vec<u64> = (0..pre_n as usize).map(|i| *pre.add(i)).collect();
    let prompt = cstr(prompt).unwrap_or_default();
    xasync::run_with(block, OP_PICKER, Box::new(move |c| {
        let items: Vec<String> = players.iter().map(|&x| gamertag_for(x)).collect();
        let preselected: Vec<usize> = players.iter().enumerate().filter(|(_, x)| presel.contains(x)).map(|(i, _)| i).collect();
        let d = Dialog {
            title: "Select players".into(),
            text: prompt,
            list: Some(List { items, multi: max > 1, preselected, min: min as usize, max: max as usize }),
            buttons: vec!["Done".into(), "Cancel".into()],
            default: 0,
            cancel: Some(1),
            ..Default::default()
        };
        let o = show(d, &c);
        if c.is_set() || o.button != Some(0) {
            return Payload::status(E_ABORT);
        }
        let mut b = Vec::new();
        for i in o.selected {
            if let Some(x) = players.get(i) {
                b.extend_from_slice(&x.to_le_bytes());
            }
        }
        Payload::data(b)
    }))
}

unsafe extern "system" fn picker_count(_t: This, block: *mut c_void, count: *mut u32) -> i32 {
    if count.is_null() {
        return E_INVALIDARG;
    }
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr >= 0 {
        *count = (n / 8) as u32;
    }
    hr
}

unsafe extern "system" fn picker_result(_t: This, block: *mut c_void, count: u32, out: *mut u64, used: *mut u32) -> i32 {
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr < 0 {
        return hr;
    }
    if (count as usize) * 8 < n || (out.is_null() && n > 0) {
        return super::E_NOT_SUFFICIENT_BUFFER;
    }
    let hr = xasync::bytes(block, OP_PICKER, n, out as *mut c_void, core::ptr::null_mut());
    if hr >= 0 && !used.is_null() {
        *used = (n / 8) as u32;
    }
    hr
}

static NOTIFICATION_HINT: AtomicU32 = AtomicU32::new(0);

pub fn notification_hint() -> u32 {
    NOTIFICATION_HINT.load(Ordering::Relaxed)
}

unsafe extern "system" fn set_notification_hint(_t: This, pos: u32) -> i32 {
    if pos > 8 {
        return E_INVALIDARG;
    }
    NOTIFICATION_HINT.store(pos, Ordering::Relaxed);
    S_OK
}

pub fn text_entry(title: String, desc: String, default: String, scope: u32, max_len: u32) -> impl FnOnce(Arc<Cancel>) -> Payload + Send {
    move |c| {
        let e = Edit { default, max: max_len, password: scope == 31, numeric: matches!(scope, 29 | 32 | 42), multiline: false };
        let d = Dialog { title, text: desc, edit: Some(e), buttons: vec!["OK".into(), "Cancel".into()], default: 0, cancel: Some(1), ..Default::default() };
        let o = show(d, &c);
        if c.is_set() || o.button != Some(0) {
            return Payload::status(E_ABORT);
        }
        let mut t = o.text;
        if max_len > 0 {
            while t.chars().count() > max_len as usize {
                t.pop();
            }
        }
        let mut b = t.into_bytes();
        b.push(0);
        Payload::data(b)
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn text_entry_async(_t: This, block: *mut c_void, title: *const u8, desc: *const u8, default: *const u8, scope: u32, max_len: u32) -> i32 {
    let (t, d, def) = (cstr(title).unwrap_or_default(), cstr(desc).unwrap_or_default(), cstr(default).unwrap_or_default());
    log("XGameUiShowTextEntryAsync", &format!("'{t}' scope={scope} max={max_len}"));
    xasync::run_with(block, OP_TEXT, Box::new(text_entry(t, d, def, scope, max_len)))
}

unsafe extern "system" fn result_size(_t: This, block: *mut c_void, size: *mut usize) -> i32 {
    xasync::size(block, size)
}

pub unsafe extern "system" fn text_entry_size(_t: This, block: *mut c_void, size: *mut u32) -> i32 {
    if size.is_null() {
        return super::E_INVALIDARG;
    }
    let mut n = 0usize;
    let hr = xasync::size(block, &mut n);
    if hr >= 0 {
        *size = n as u32;
    }
    hr
}

pub unsafe extern "system" fn text_entry_result(_t: This, block: *mut c_void, size: u32, buf: *mut u8, used: *mut u32) -> i32 {
    let mut n = 0usize;
    let hr = xasync::bytes(block, OP_TEXT, size as usize, buf as *mut c_void, &mut n);
    if !used.is_null() {
        *used = n as u32;
    }
    hr
}

unsafe extern "system" fn web_auth_result(_t: This, block: *mut c_void, size: usize, buf: *mut u8, ptr: *mut *mut u8, used: *mut usize) -> i32 {
    if !ptr.is_null() {
        *ptr = core::ptr::null_mut();
    }
    let hr = xasync::bytes(block, OP_WEB, size, buf as *mut c_void, used);
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}

fn web_auth_payload(r: Result<(String, u32), i32>) -> Payload {
    match r {
        Ok((uri, status)) => {
            let mut b = vec![0u8; 16];
            b[0..8].copy_from_slice(&16u64.to_le_bytes());
            b[8..12].copy_from_slice(&status.to_le_bytes());
            b.extend_from_slice(uri.as_bytes());
            b.push(0);
            Payload { hr: S_OK, bytes: b, fixups: vec![0] }
        }
        Err(hr) => Payload::status(hr),
    }
}

unsafe fn web_auth(block: *mut c_void, uri: *const u8, done: *const u8) -> i32 {
    let (Some(u), Some(d)) = (cstr(uri), cstr(done)) else { return E_INVALIDARG };
    log("XGameUiShowWebAuthenticationAsync", &format!("'{u}' until '{d}'"));
    xasync::run_with(block, OP_WEB, Box::new(move |c| web_auth_payload(super::webauth::run(&u, &d, &c))))
}

unsafe extern "system" fn web_auth_async(_t: This, block: *mut c_void, _u: usize, uri: *const u8, done: *const u8) -> i32 {
    web_auth(block, uri, done)
}

unsafe extern "system" fn web_auth_options_async(_t: This, block: *mut c_void, _u: usize, uri: *const u8, done: *const u8, _opts: u32) -> i32 {
    web_auth(block, uri, done)
}

fn v1_slots() -> Vec<(usize, usize)> {
    vec![
        (3, message_async as usize),
        (4, message_result as usize),
        (5, invite_async as usize),
        (6, status_result as usize),
        (7, profile_async as usize),
        (8, status_result as usize),
        (9, achievements_async as usize),
        (10, status_result as usize),
        (17, text_entry_async as usize),
        (18, text_entry_size as usize),
        (19, text_entry_result as usize),
    ]
}

pub fn interfaces() -> Vec<Iface> {
    let v1 = v1_slots();
    let mut v2 = v1_slots();
    v2.extend([
        (11, picker_async as usize),
        (12, picker_count as usize),
        (13, picker_result as usize),
        (14, error_async as usize),
        (15, status_result as usize),
        (16, set_notification_hint as usize),
        (24, web_auth_async as usize),
        (25, result_size as usize),
        (26, web_auth_result as usize),
        (27, web_auth_options_async as usize),
        (30, mpa_invite_async as usize),
        (31, status_result as usize),
    ]);
    vec![
        Iface { clsid: CLSID, iid: IID_V1, name: "XGameUi(6eeaa73e)", slots: v1 },
        Iface { clsid: CLSID, iid: IID_V2, name: "XGameUi(72c4e70f)", slots: v2.clone() },
        Iface { clsid: CLSID, iid: IID_FC0B, name: "XGameUi(fc0b9bd0)", slots: v2 },
    ]
}
