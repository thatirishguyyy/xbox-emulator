use std::time::Instant;
use tombox_xshader::shader_cache::Fma;
use tombox_xshader::{dxc, extract, hlsl, runtime_hlsl};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut files = Vec::new();
    let mut fma = Fma::default();
    let mut save: Option<String> = None;
    let mut list: Option<String> = None;
    let mut xlat_only = false;
    let mut extra: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fma" => {
                fma = match args[i + 1].as_str() {
                    "on" => Fma { preserve: true, any: true, hybrid: false, f2h: false },
                    "probe" => tombox_xshader::fma_probe::probe().fma,
                    _ => Fma::default(),
                };
                i += 2;
            }
            "--save" => {
                save = Some(args[i + 1].clone());
                i += 2;
            }
            "--list" => {
                list = Some(args[i + 1].clone());
                i += 2;
            }
            "--arg" => {
                extra.push(args[i + 1].clone());
                i += 2;
            }
            "--xlat" => {
                xlat_only = true;
                i += 1;
            }
            _ => {
                files.push(args[i].clone());
                i += 1;
            }
        }
    }
    if let Some(l) = &list {
        files.extend(std::fs::read_to_string(l).expect("list").lines().filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()));
    }
    let ex: Vec<&str> = extra.iter().map(|s| s.as_str()).collect();
    println!("fma {fma:?} extra {ex:?}");
    let (mut total, mut n) = (0f64, 0usize);
    for f in &files {
        let stem = std::path::Path::new(f).file_stem().unwrap().to_string_lossy().into_owned();
        let blob = std::fs::read(f).expect("read");
        let t0 = Instant::now();
        let t = match extract(&blob).map_err(|e| e.to_string()).and_then(|e| hlsl::translate(&e).map_err(|e| e.to_string())) {
            Ok(t) => t,
            Err(e) => {
                println!("{stem}: translate: {e}");
                continue;
            }
        };
        let xl = t0.elapsed().as_secs_f64() * 1000.0;
        let src = runtime_hlsl::finalize(&t.hlsl, &[0; 8], true);
        let profile = dxc::profile_for(t.profile);
        if let Some(d) = &save {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(std::path::Path::new(d).join(format!("{stem}.hlsl")), &src);
        }
        if xlat_only {
            println!("{stem}: {profile} xlat {xl:.0} ms lines {} bytes {} fma {} rto {} dbl {}", src.lines().count(), src.len(), src.matches("tb_fma(").count(), src.matches("tb_rto(").count(), src.matches("double").count());
            continue;
        }
        let t1 = Instant::now();
        let r = dxc::compile_fma(&src, &profile, fma, &ex);
        let ms = t1.elapsed().as_secs_f64() * 1000.0;
        total += ms;
        n += 1;
        match &r {
            Ok(code) => {
                if let Some(d) = &save {
                    let _ = std::fs::write(std::path::Path::new(d).join(format!("{stem}.dxil")), code);
                }
                println!("{stem}: {profile} xlat {xl:.0} ms dxc {ms:.0} ms lines {} dxil {}", src.lines().count(), code.len())
            }
            Err(e) => println!("{stem}: {profile} FAIL {ms:.0} ms: {e}"),
        }
    }
    println!("total dxc {total:.0} ms over {n} shaders");
}
