use super::*;

#[link(name = "ntdll")]
extern "system" {
    fn AlpcInitializeMessageAttribute(flags: u32, buf: *mut u8, len: usize, req: *mut usize) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreatePipe(r: *mut usize, w: *mut usize, sa: *const c_void, size: u32) -> i32;
    fn WriteFile(h: usize, b: *const u8, n: u32, done: *mut u32, ov: *mut c_void) -> i32;
    fn ReadFile(h: usize, b: *mut u8, n: u32, done: *mut u32, ov: *mut c_void) -> i32;
}

struct Name {
    _w: Vec<u16>,
    us: [usize; 2],
}

fn name(s: &str) -> Box<Name> {
    let w: Vec<u16> = s.encode_utf16().collect();
    let len = (w.len() * 2) as usize;
    let mut n = Box::new(Name { _w: w, us: [0; 2] });
    n.us = [len | (len << 16), n._w.as_ptr() as usize];
    n
}

fn oa(n: &Name) -> [usize; 6] {
    [0x30, 0, n.us.as_ptr() as usize, 0, 0, 0]
}

fn attrs(max_message: usize, max_section: usize) -> [u8; 0x48] {
    let mut a = [0u8; 0x48];
    a[0x10..0x18].copy_from_slice(&max_message.to_le_bytes());
    a[0x20..0x28].copy_from_slice(&0x4000usize.to_le_bytes());
    a[0x28..0x30].copy_from_slice(&max_section.to_le_bytes());
    a[0x30..0x38].copy_from_slice(&max_section.to_le_bytes());
    a[0x38..0x40].copy_from_slice(&max_section.to_le_bytes());
    a
}

fn msg_attrs(flags: u32) -> Vec<u8> {
    let mut need = 0usize;
    unsafe { AlpcInitializeMessageAttribute(flags, core::ptr::null_mut(), 0, &mut need) };
    let mut b = vec![0u8; need.max(8)];
    let mut got = 0usize;
    assert_eq!(unsafe { AlpcInitializeMessageAttribute(flags, b.as_mut_ptr(), b.len(), &mut got) }, 0);
    b
}

fn message(data: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; HEADER + data.len()];
    m[0..2].copy_from_slice(&(data.len() as u16).to_le_bytes());
    m[2..4].copy_from_slice(&((HEADER + data.len()) as u16).to_le_bytes());
    m[HEADER..].copy_from_slice(data);
    m
}

unsafe fn receive(port: usize, attrs: &mut [u8]) -> (i32, Vec<u8>) {
    let mut buf = vec![0u8; 0x100];
    let mut len = buf.len();
    let st = nt_alpc_send_wait_receive_port(port, 0, core::ptr::null(), core::ptr::null_mut(), buf.as_mut_ptr(), &mut len, attrs.as_mut_ptr(), core::ptr::null());
    buf.truncate(len);
    (st, buf)
}

#[test]
fn connect_accept_view_handles_request_reply_and_close() {
    unsafe {
        let n = name("\\TomBoxAlpcModelTest");
        let o = oa(&n);
        let a = attrs(0x40, 0x10000);
        let mut server = 0usize;
        assert_eq!(nt_alpc_create_port(&mut server, o.as_ptr() as *const u8, a.as_ptr()), 0);
        let mut again = 0usize;
        assert_eq!(nt_alpc_create_port(&mut again, o.as_ptr() as *const u8, a.as_ptr()), STATUS_OBJECT_NAME_COLLISION);
        assert!(port_exists("\\tomboxalpcmodeltest"));
        let srv = std::thread::spawn(move || {
            let mut ra = msg_attrs(ATTR_VIEW | ATTR_CONTEXT | ATTR_HANDLE);
            let (st, conn) = receive(server, &mut ra);
            assert_eq!(st, 0);
            assert_eq!(conn[4], LPC_CONNECTION_REQUEST as u8);
            assert_eq!(&conn[HEADER..HEADER + 4], b"helo");
            let mut proc_h = 0usize;
            assert_eq!(nt_alpc_open_sender_process(&mut proc_h, server, conn.as_ptr(), 0, 0x0200_0000, core::ptr::null()), 0);
            assert_ne!(proc_h, 0);
            let mut sec = 0usize;
            let mut actual = 0usize;
            assert_eq!(nt_alpc_create_port_section(server, 0, 0, 0x20000, &mut sec, &mut actual), STATUS_SECTION_TOO_BIG);
            assert_eq!(nt_alpc_create_port_section(server, 0, 0, 0x8000, &mut sec, &mut actual), 0);
            assert_eq!(actual, 0x10000);
            let va = AlpcGetMessageAttribute(ra.as_mut_ptr(), ATTR_VIEW);
            (va as *mut u32).write(0);
            (va.add(8) as *mut usize).write(sec);
            (va.add(0x10) as *mut usize).write(0);
            (va.add(0x18) as *mut usize).write(actual);
            assert_eq!(nt_alpc_create_section_view(server, 0, va), 0);
            let base = (va.add(0x10) as *const usize).read();
            *(base as *mut u8) = 0x5a;
            let (mut r1, mut w1, mut r2, mut w2) = (0usize, 0usize, 0usize, 0usize);
            assert_ne!(CreatePipe(&mut r1, &mut w1, core::ptr::null(), 0), 0);
            assert_ne!(CreatePipe(&mut r2, &mut w2, core::ptr::null(), 0), 0);
            let list: [u32; 8] = [0, r1 as u32, 0, 0x8000_0000, 0, w2 as u32, 0, 0x4000_0000];
            let ha = AlpcGetMessageAttribute(ra.as_mut_ptr(), ATTR_HANDLE);
            (ha as *mut u32).write(HANDLEFLG_INDIRECT);
            (ha.add(8) as *mut usize).write(list.as_ptr() as usize);
            (ha.add(0x10) as *mut u32).write(2);
            (ra.as_mut_ptr().add(4) as *mut u32).write(ATTR_VIEW | ATTR_HANDLE);
            let mut reply = conn.clone();
            reply[HEADER + 8] = 7;
            let mut comm = 0usize;
            let context = 0x1234_5678usize;
            assert_eq!(nt_alpc_accept_connect_port(&mut comm, server, 0, core::ptr::null(), core::ptr::null(), context, reply.as_ptr(), ra.as_mut_ptr(), 1), 0);
            crate::nt_close(r1);
            crate::nt_close(w2);
            assert!(WriteFile(w1, b"event!".as_ptr(), 6, &mut 0, core::ptr::null_mut()) != 0);
            let mut ca = msg_attrs(ATTR_CONTEXT);
            let (st, req) = receive(server, &mut ca);
            assert_eq!(st, 0);
            assert_eq!(req[4], LPC_REQUEST as u8);
            let ctx = AlpcGetMessageAttribute(ca.as_mut_ptr(), ATTR_CONTEXT);
            assert_eq!((ctx as *const usize).read(), context);
            let mut rep = req.clone();
            rep[HEADER + 4] = req[HEADER] + 1;
            assert_eq!(nt_alpc_send_wait_receive_port(server, 0x10000, rep.as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), 0);
            let (st, closed) = receive(server, &mut ca);
            assert_eq!(st, 0);
            assert_eq!(closed[4], LPC_PORT_CLOSED as u8);
            assert_eq!((AlpcGetMessageAttribute(ca.as_mut_ptr(), ATTR_CONTEXT) as *const usize).read(), context);
            assert_eq!(nt_alpc_delete_section_view(server, 0, base), 0);
            assert_eq!(nt_alpc_delete_port_section(server, 0, sec), 0);
            assert_eq!(nt_alpc_disconnect_port(comm, 0), 0);
            crate::nt_close(comm);
            crate::nt_close(proc_h);
            let mut ack = [0u8; 3];
            assert!(ReadFile(r2, ack.as_mut_ptr(), 3, &mut 0, core::ptr::null_mut()) != 0);
            assert_eq!(&ack, b"ack");
            crate::nt_close(r2);
            crate::nt_close(w1);
            server
        });
        let mut ca = attrs(0x40, 0);
        ca[0x40] = 1;
        let mut connect = message(b"helo\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        let mut len = 0x40usize;
        let mut ia = msg_attrs(ATTR_VIEW | ATTR_CONTEXT | ATTR_HANDLE);
        let mut client = 0usize;
        let st = nt_alpc_connect_port(&mut client, n.us.as_ptr() as *const u8, core::ptr::null(), ca.as_ptr(), 0x20000, core::ptr::null(), connect.as_mut_ptr(), &mut len, core::ptr::null_mut(), ia.as_mut_ptr(), core::ptr::null());
        assert_eq!(st, 0);
        assert_eq!(connect[HEADER + 8], 7);
        let valid = (ia.as_ptr().add(4) as *const u32).read();
        assert_eq!(valid & (ATTR_VIEW | ATTR_HANDLE), ATTR_VIEW | ATTR_HANDLE);
        let ha = AlpcGetMessageAttribute(ia.as_mut_ptr(), ATTR_HANDLE);
        assert_eq!((ha.add(0x10) as *const u32).read(), 2);
        let va = AlpcGetMessageAttribute(ia.as_mut_ptr(), ATTR_VIEW);
        assert_eq!((va.add(0x18) as *const usize).read(), 0x10000);
        assert_eq!(*((va.add(0x10) as *const usize).read() as *const u8), 0x5a);
        let mut info = [0u32; 5];
        assert_eq!(nt_alpc_query_information_message(client, connect.as_ptr(), MESSAGE_HANDLE_INFORMATION, info.as_mut_ptr() as *mut u8, 0x14, core::ptr::null_mut()), 0);
        let events = info[2] as usize;
        info = [1, 0, 0, 0, 0];
        assert_eq!(nt_alpc_query_information_message(client, connect.as_ptr(), MESSAGE_HANDLE_INFORMATION, info.as_mut_ptr() as *mut u8, 0x14, core::ptr::null_mut()), 0);
        let acks = info[2] as usize;
        info = [2, 0, 0, 0, 0];
        assert_eq!(nt_alpc_query_information_message(client, connect.as_ptr(), MESSAGE_HANDLE_INFORMATION, info.as_mut_ptr() as *mut u8, 0x14, core::ptr::null_mut()), STATUS_INVALID_PARAMETER);
        let mut ev = [0u8; 6];
        assert!(ReadFile(events, ev.as_mut_ptr(), 6, &mut 0, core::ptr::null_mut()) != 0);
        assert_eq!(&ev, b"event!");
        assert!(WriteFile(acks, b"ack".as_ptr(), 3, &mut 0, core::ptr::null_mut()) != 0);
        let big = message(&[0u8; 0x20]);
        let mut out = vec![0u8; 0x80];
        let mut olen = out.len();
        assert_eq!(nt_alpc_send_wait_receive_port(client, 0, big.as_ptr(), core::ptr::null_mut(), out.as_mut_ptr(), &mut olen, core::ptr::null_mut(), core::ptr::null()), STATUS_PORT_MESSAGE_TOO_LONG);
        let req = message(&[41u8; 0x18]);
        let mut rep = vec![0u8; 0x40];
        let mut rlen = rep.len();
        assert_eq!(nt_alpc_send_wait_receive_port(client, 0, req.as_ptr(), core::ptr::null_mut(), rep.as_mut_ptr(), &mut rlen, core::ptr::null_mut(), core::ptr::null()), 0);
        assert_eq!(rlen, 0x40);
        assert_eq!(rep[4], LPC_REPLY as u8);
        assert_eq!(rep[HEADER + 4], 42);
        crate::nt_close(events);
        crate::nt_close(acks);
        crate::nt_close(client);
        let server = srv.join().unwrap();
        assert_eq!(connection_count("\\TomBoxAlpcModelTest"), 0);
        assert_eq!(section_count("\\TomBoxAlpcModelTest"), 0);
        crate::nt_close(server);
        assert!(!port_exists("\\TomBoxAlpcModelTest"));
    }
}

#[test]
fn refused_unknown_and_denied_connections() {
    unsafe {
        let n = name("\\TomBoxAlpcRefuse");
        let o = oa(&n);
        let mut server = 0usize;
        assert_eq!(nt_alpc_create_port(&mut server, o.as_ptr() as *const u8, core::ptr::null()), 0);
        let srv = std::thread::spawn(move || {
            let mut ra = msg_attrs(ATTR_CONTEXT);
            let (st, conn) = receive(server, &mut ra);
            assert_eq!(st, 0);
            assert_eq!(nt_alpc_accept_connect_port(core::ptr::null_mut(), server, 0, core::ptr::null(), core::ptr::null(), 0, conn.as_ptr(), core::ptr::null_mut(), 0), 0);
            server
        });
        let mut client = 0usize;
        assert_eq!(nt_alpc_connect_port(&mut client, n.us.as_ptr() as *const u8, core::ptr::null(), core::ptr::null(), 0x20000, core::ptr::null(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), STATUS_PORT_CONNECTION_REFUSED);
        let server = srv.join().unwrap();
        let missing = name("\\TomBoxAlpcNobody");
        assert_eq!(nt_alpc_connect_port(&mut client, missing.us.as_ptr() as *const u8, core::ptr::null(), core::ptr::null(), 0, core::ptr::null(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), STATUS_OBJECT_NAME_NOT_FOUND);
        let timeout = -10_000i64 * 50;
        assert_eq!(nt_alpc_connect_port(&mut client, n.us.as_ptr() as *const u8, core::ptr::null(), core::ptr::null(), 0, core::ptr::null(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), &timeout), STATUS_TIMEOUT);
        let mut ra = msg_attrs(ATTR_CONTEXT);
        let mut buf = vec![0u8; 0x100];
        let mut len = buf.len();
        assert_eq!(nt_alpc_send_wait_receive_port(server, 0, core::ptr::null(), core::ptr::null_mut(), buf.as_mut_ptr(), &mut len, ra.as_mut_ptr(), &timeout), STATUS_TIMEOUT);
        crate::nt_close(server);
        let mut sd = [0u8; 40];
        let mut acl = [0u8; 8];
        acl[0] = 2;
        acl[2..4].copy_from_slice(&8u16.to_le_bytes());
        sd[0] = 1;
        sd[2..4].copy_from_slice(&0x0004u16.to_le_bytes());
        (sd.as_mut_ptr().add(0x20) as *mut usize).write_unaligned(acl.as_ptr() as usize);
        let locked = name("\\TomBoxAlpcLocked");
        let mut lo = oa(&locked);
        lo[4] = sd.as_ptr() as usize;
        let mut lp = 0usize;
        assert_eq!(nt_alpc_create_port(&mut lp, lo.as_ptr() as *const u8, core::ptr::null()), 0);
        assert_eq!(nt_alpc_connect_port(&mut client, locked.us.as_ptr() as *const u8, core::ptr::null(), core::ptr::null(), 0, core::ptr::null(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), STATUS_ACCESS_DENIED);
        crate::nt_close(lp);
    }
}
