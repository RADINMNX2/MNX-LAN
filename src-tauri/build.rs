fn main() {
    let mut attrs = tauri_build::WindowsAttributes::new();
    attrs = attrs.app_manifest(
        r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{4f476546-9370-4e2a-9d6a-8e4f0a7a9c25}" />
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" />
    </application>
  </compatibility>
</assembly>
"#,
    );
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(attrs))
        .expect("failed to run tauri-build")
}