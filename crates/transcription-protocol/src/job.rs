//! Worker lifetime bound to the parent (docs/03 §5, docs/10 §3 "exits with
//! parent"). On Windows the worker is placed in a job object with
//! `KILL_ON_JOB_CLOSE`: when the app exits or crashes, the kernel closes the
//! job handle and terminates the worker. On Unix the worker already exits on
//! stdin EOF, which the kernel delivers when the parent dies.

use std::process::Child;

#[cfg(windows)]
pub struct ParentBound(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ParentBound {
    pub fn attach(child: &Child) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        // SAFETY: anonymous job owned by `Self` and closed in Drop.
        let job = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }
            .map_err(std::io::Error::other)?;
        let bound = Self(job);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: correctly sized limit structure for this information class.
        unsafe {
            SetInformationJobObject(
                bound.0,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                u32::try_from(std::mem::size_of_val(&info)).unwrap_or(0),
            )
        }
        .map_err(std::io::Error::other)?;
        // SAFETY: the child's process handle stays valid while `child` lives.
        unsafe { AssignProcessToJobObject(bound.0, HANDLE(child.as_raw_handle())) }
            .map_err(std::io::Error::other)?;
        Ok(bound)
    }
}

#[cfg(windows)]
impl Drop for ParentBound {
    fn drop(&mut self) {
        // SAFETY: job handle created in `attach`, closed once; this kills the worker.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

// SAFETY: a job handle is a kernel handle usable from any thread.
#[cfg(windows)]
unsafe impl Send for ParentBound {}

#[cfg(not(windows))]
pub struct ParentBound;

#[cfg(not(windows))]
impl ParentBound {
    pub fn attach(_child: &Child) -> std::io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(all(test, windows))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn closing_the_job_kills_the_child() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let bound = ParentBound::attach(&child).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            child.try_wait().unwrap().is_none(),
            "child should be running"
        );
        drop(bound);
        let t = std::time::Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                t.elapsed() < std::time::Duration::from_secs(5),
                "child survived job close"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
