use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/teekasfigure.ico");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/teekasfigure.ico");
    let rc = out.join("teekasfigure.rc");
    // rc.exe can mishandle non-ASCII absolute asset paths. Compile relative to
    // OUT_DIR using ASCII file names (the Rust project may live under Cyrillic).
    fs::copy(&icon, out.join("teekasfigure.ico")).unwrap();
    fs::write(
        &rc,
        format!(
            r#"1 ICON "{}"
1 VERSIONINFO
FILEVERSION 0,1,0,0
PRODUCTVERSION 0,1,0,0
FILEOS 0x40004
FILETYPE 1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "TeekasFigure"
      VALUE "ProductName", "TeekasFigure"
      VALUE "OriginalFilename", "TeekasFigure.exe"
      VALUE "FileVersion", "0.1.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
            "teekasfigure.ico"
        ),
    )
    .unwrap();
    let resource = out.join("teekasfigure.res");
    let compiler = env::var_os("RC")
        .map(PathBuf::from)
        .or_else(|| {
            let kits = PathBuf::from(env::var_os("ProgramFiles(x86)")?).join("Windows Kits/10/bin");
            let mut versions: Vec<_> = fs::read_dir(kits)
                .ok()?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .collect();
            versions.sort();
            versions
                .into_iter()
                .rev()
                .map(|p| p.join("x64/rc.exe"))
                .find(|p| p.is_file())
        })
        .unwrap_or_else(|| "rc.exe".into());
    let mut command = Command::new(compiler);
    command
        .current_dir(&out)
        .arg("/nologo")
        .arg("/fo")
        .arg("teekasfigure.res")
        .arg("teekasfigure.rc");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command
        .output()
        .expect("Windows SDK Resource Compiler (rc.exe) is required to embed the application icon");
    assert!(
        result.status.success(),
        "Resource compiler failed: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!(
        "cargo:rustc-link-arg-bin=gpu-image-approximator={}",
        resource.display()
    );
}
