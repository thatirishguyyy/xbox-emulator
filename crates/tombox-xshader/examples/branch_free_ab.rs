use std::time::Instant;
use tombox_xshader::{branch_free, dxc};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reps: usize = std::env::var("REPS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    for pair in args.chunks(2) {
        let src = std::fs::read_to_string(&pair[0]).expect("read");
        let profile = pair.get(1).cloned().unwrap_or_else(|| "ps_6_2".into());
        let t = Instant::now();
        let bf = branch_free::rewrite(&src);
        let rw = t.elapsed().as_secs_f64() * 1000.0;
        let (mut a, mut b) = (f64::MAX, f64::MAX);
        for _ in 0..reps {
            let t = Instant::now();
            let ra = dxc::compile_denorm(&src, &profile, "preserve", &["-D", "TB_FAST_FMA"]);
            a = a.min(t.elapsed().as_secs_f64() * 1000.0);
            let t = Instant::now();
            let rb = dxc::compile_denorm(&bf, &profile, "preserve", &["-D", "TB_FAST_FMA"]);
            b = b.min(t.elapsed().as_secs_f64() * 1000.0);
            if ra.is_err() || rb.is_err() {
                println!("{}: compile error orig={} bf={}", pair[0], ra.err().unwrap_or_default().lines().next().unwrap_or(""), rb.err().unwrap_or_default().lines().next().unwrap_or(""));
                break;
            }
        }
        println!("{} {profile}: original {a:.0} ms, branch-free {b:.0} ms, rewrite {rw:.1} ms, dll {}", pair[0], dxc::identity());
    }
}
