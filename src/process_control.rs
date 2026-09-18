use windows::Win32::{
    Foundation::CloseHandle,
    System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
};

/// Requests termination of a process selected by the user after confirmation.
/// Windows access control remains authoritative for every other process.
pub fn terminate_process(pid: u32) -> Result<(), String> {
    if matches!(pid, 0 | 4) {
        return Err("wtop will not terminate a core Windows process".into());
    }
    if pid == std::process::id() {
        return Err("wtop will not terminate itself".into());
    }

    let handle =
        unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }.map_err(|error| error.to_string())?;
    let result = unsafe { TerminateProcess(handle, 1) }.map_err(|error| error.to_string());
    let close_result = unsafe { CloseHandle(handle) }.map_err(|error| error.to_string());

    match (result, close_result) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::terminate_process;

    #[test]
    fn core_processes_are_rejected_before_opening_a_handle() {
        assert!(terminate_process(0).is_err());
        assert!(terminate_process(4).is_err());
    }
}
