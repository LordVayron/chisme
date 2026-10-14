//! Stream keys in Windows Credential Manager.
//! They never go into settings.json.

use anyhow::Result;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};
use windows::core::{HSTRING, PWSTR};

fn target(dest: &str) -> HSTRING {
    HSTRING::from(format!("Chisme stream key: {}", dest.trim().to_lowercase()))
}

pub fn get(dest: &str) -> Option<String> {
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        CredReadW(&target(dest), CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let c = &*p;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let key = String::from_utf8(bytes.to_vec()).ok();
        CredFree(p as _);
        key.filter(|k| !k.is_empty())
    }
}

pub fn set(dest: &str, key: &str) -> Result<()> {
    let name = target(dest);
    let user = HSTRING::from("chisme");
    let mut blob = key.as_bytes().to_vec();
    let cred = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_ptr() as *mut _),
        Comment: PWSTR::null(),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user.as_ptr() as *mut _),
    };
    unsafe { CredWriteW(&cred, 0)? };
    Ok(())
}

pub fn delete(dest: &str) -> Result<()> {
    unsafe { CredDeleteW(&target(dest), CRED_TYPE_GENERIC, None)? };
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let dest = format!("test-{}", std::process::id());
        super::set(&dest, "live_abc123").unwrap();
        assert_eq!(super::get(&dest).as_deref(), Some("live_abc123"));
        super::delete(&dest).unwrap();
        assert_eq!(super::get(&dest), None);
    }
}
