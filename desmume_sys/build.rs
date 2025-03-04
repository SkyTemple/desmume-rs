use std::env;
use std::error::Error;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn env(name: &str) -> Option<String> {
    let prefix = env::var("TARGET").unwrap().to_uppercase().replace('-', "_");
    let prefixed = format!("{}_{}", prefix, name);
    println!("cargo:rerun-if-env-changed={}", prefixed);

    if let Ok(var) = env::var(&prefixed) {
        return Some(var);
    }

    println!("cargo:rerun-if-env-changed={}", name);
    env::var(name).ok()
}

fn main() {
    if env("_CI_ONLY__DISABLE_DESMUME_SYS_BUILD_SCRIPT").is_some() {
        return;
    }

    let target = env::var("TARGET").unwrap();

    let src = env::current_dir().unwrap();
    let build_dir = TempDir::new().unwrap();
    let build_dir = build_dir.path();

    // Copy sources over
    let mut cmd = Command::new("cp");
    cmd.current_dir(build_dir)
        .arg("-a")
        .arg(src.join("desmume/desmume/src"))
        .arg(build_dir);
    run(&mut cmd, "cp");

    if target.contains("windows") {
        // MSVC-based Windows build
        let mut cmd = Command::new("MSBuild.exe");
        let config = "Release Fastbuild";
        let (arch_dirname, arch_targetname) = if cfg!(target_pointer_width = "64") {
            ("x64", "x64")
        } else {
            ("x86", "Win32")
        };

        // before we continue, we need to make sure that we do not set WIN_EXPORT
        // which is set by default because the default is building a dynamic libary:
        let props_path = build_dir.join("src/frontend/interface/windows/desmume.props");
        let props = fs::read_to_string(&props_path).unwrap();
        let mprops = props.replace(";WIN_EXPORT", "");
        fs::write(props_path, mprops).unwrap();

        cmd.arg("DeSmuME_Interface.vcxproj")
            .arg(format!("/p:configuration={}", config))
            .arg(format!("/p:Platform={}", arch_targetname))
            .arg("-property:ConfigurationType=StaticLibrary")
            .current_dir(build_dir.join("src/frontend/interface/windows"));
        run(&mut cmd, "MSBuild");

        let dst = PathBuf::from(env::var_os("OUT_DIR").unwrap());

        windows_move_libs(
            build_dir,
            "src/frontend/interface/windows/__bins/*.lib",
            &dst,
        )
        .unwrap();
        windows_move_libs(
            build_dir,
            "src/frontend/interface/windows/.libs/*.lib",
            &dst,
        )
        .unwrap();
        windows_move_libs(
            build_dir,
            format!("src/frontend/interface/windows/.libs/{arch_targetname}/*.lib"),
            &dst,
        )
        .unwrap();
        windows_move_libs(
            build_dir,
            format!("src/frontend/interface/windows/SDL/lib/{arch_dirname}/*.lib"),
            &dst,
        )
        .unwrap();
        windows_move_libs(build_dir, "src/frontend/windows/zlib128/*.lib", &dst).unwrap();
        windows_move_libs(build_dir, "src/frontend/windows/agg/*.lib", &dst).unwrap();

        println!(
            "cargo:rustc-link-search={}",
            dst.as_os_str().to_str().unwrap()
        );

        println!("cargo:rustc-link-lib=dylib=vfw32");
        println!("cargo:rustc-link-lib=dylib=opengl32");
        println!("cargo:rustc-link-lib=dylib=glu32");
        println!("cargo:rustc-link-lib=dylib=ws2_32");
        println!("cargo:rustc-link-lib=dylib=user32");
        println!("cargo:rustc-link-lib=dylib=gdi32");
        println!("cargo:rustc-link-lib=dylib=shell32");
        println!("cargo:rustc-link-lib=dylib=comdlg32");
        println!("cargo:rustc-link-lib=dylib=shlwapi");
        println!("cargo:rustc-link-lib=dylib=comctl32");
        println!("cargo:rustc-link-lib=dylib=winmm");
        // NOTE: The static lib also just contains stubs that call into the DLL. The DLL is needed
        // to run!
        println!("cargo:rustc-link-lib=static=SDL2");
        if arch_targetname == "x64" {
            println!("cargo:rustc-link-lib=static=agg-2.5-x64");
            println!("cargo:rustc-link-lib=static=zlib-vc8-x64");
        } else {
            println!("cargo:rustc-link-lib=static=agg-2.5");
            println!("cargo:rustc-link-lib=static=zlib-vc8-Win32");
        }

        let desmume_lib_name = glob::glob(dst.join("DeSmuME*").to_str().unwrap())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        fs::rename(desmume_lib_name, dst.join("desmume.lib")).unwrap();

        println!("cargo:rustc-link-lib=static=desmume");

        // Needed for 32bit version of zlib
        if arch_targetname != "x64" {
            println!("cargo:rustc-link-arg=/SAFESEH:NO");
        }
    } else {
        // Meson based Linux/Mac build
        let mut cmd = Command::new("meson");
        cmd.arg("build")
            .arg("--default-library=static")
            .arg("-Dbuildtype=release")
            // Prevent implicit-function-declaration and int-conversion errors
            .arg("-Dc_std=gnu11")
            .arg("-Dcpp_std=gnu++14")
            .current_dir(build_dir.join("src/frontend/interface"));
        run(&mut cmd, "meson");

        let mut cmd = Command::new("ninja");
        cmd.arg("-C")
            .arg("build")
            .current_dir(build_dir.join("src/frontend/interface"));
        run(&mut cmd, "ninja");

        let dst = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let mut cmd = Command::new("cp");
        cmd.current_dir(build_dir)
            .arg("-r")
            .arg(build_dir.join("src/frontend/interface/build/libdesmume.a"))
            .arg(build_dir.join("src/frontend/interface/build/libdesmume.a.p"))
            .arg(&dst);
        run(&mut cmd, "cp");

        let cfg = pkg_config::Config::new();
        cfg.probe("glib-2.0").unwrap();
        cfg.probe("sdl2").unwrap();
        if cfg.probe("libpcap").is_err() {
            if target.contains("darwin") {
                // Probing may fail under MacOS. Still try to link.
                println!("cargo:rustc-link-lib=pcap");
            } else {
                panic!("Could not find libpcap. Is it installed?");
            }
        }
        cfg.probe("zlib").unwrap();
        cfg.probe("soundtouch").ok();
        cfg.probe("openal").ok();
        if cfg.probe("opengl").is_err() && target.contains("darwin") {
            // Probing may fail under MacOS. Still try to link.
            println!("cargo:rustc-link-lib=framework=OpenGL");
        }
        cfg.probe("alsa").ok();

        if target.contains("darwin") {
            println!("cargo:rustc-link-lib=c++");
        } else {
            println!("cargo:rustc-link-lib=stdc++");
        }
        println!("cargo:rustc-link-search={}", dst.display());
    }
}

fn run(cmd: &mut Command, program: &str) {
    println!("running: {:?}", cmd);
    let status = match cmd.status() {
        Ok(status) => status,
        Err(ref e) if e.kind() == ErrorKind::NotFound => {
            fail(&format!(
                "failed to execute command: {}\nis `{}` not installed?",
                e, program
            ));
        }
        Err(e) => fail(&format!("failed to execute command: {}", e)),
    };
    if !status.success() {
        fail(&format!(
            "command did not execute successfully, got: {}",
            status
        ));
    }
}

fn fail(s: &str) -> ! {
    panic!("\n{}\n\nbuild script failed, must exit now", s)
}

fn windows_move_libs<P: AsRef<Path>>(
    build_dir: &Path,
    glob_pattern: P,
    dst: &Path,
) -> Result<(), Box<dyn Error>> {
    let gl = glob::glob(build_dir.join(glob_pattern).to_str().unwrap())?;
    for pathr in gl {
        let path = pathr?;
        fs::copy(&path, dst.join(path.file_name().unwrap()))?;
    }
    Ok(())
}
