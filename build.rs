use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=assets/dayring.ico");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap().replace('\\', "/");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let [major, minor, patch] = ["MAJOR", "MINOR", "PATCH"]
        .map(|key| env::var(format!("CARGO_PKG_VERSION_{key}")).unwrap());

    let manifest_path = Path::new(&out_dir).join("dayring.manifest");
    fs::write(
        &manifest_path,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="dayring" version="0.1.0.0" />
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>
"#,
    )
    .unwrap();
    let manifest = manifest_path.display().to_string().replace('\\', "/");

    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{manifest_dir}/assets/dayring.ico"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "dayring"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "dayring"
      VALUE "OriginalFilename", "dayring.exe"
      VALUE "ProductName", "dayring"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let rc_path = Path::new(&out_dir).join("dayring.rc");
    fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_required()
        .expect("embed dayring icon and version resource");
}
