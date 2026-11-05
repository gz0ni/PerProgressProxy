pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl LocalTime {
    pub fn hms(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    pub fn full(&self) -> String {
        format!("{:04}-{:02}-{:02} {}", self.year, self.month, self.day, self.hms())
    }
}

#[cfg(windows)]
pub fn local_now() -> LocalTime {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    LocalTime {
        year: t.wYear as i32,
        month: t.wMonth as u32,
        day: t.wDay as u32,
        hour: t.wHour as u32,
        minute: t.wMinute as u32,
        second: t.wSecond as u32,
    }
}

#[cfg(unix)]
pub fn local_now() -> LocalTime {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        LocalTime {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            second: tm.tm_sec as u32,
        }
    }
}

#[cfg(windows)]
pub fn install_interrupt_handler() {
    use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT};

    unsafe extern "system" fn handler(kind: u32) -> i32 {
        if kind == CTRL_C_EVENT {
            println!("\ninterrupted");
            std::process::exit(130);
        }
        0
    }
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(unix)]
pub fn install_interrupt_handler() {
    extern "C" fn handler(_: libc::c_int) {
        let message = b"\ninterrupted\n";
        unsafe {
            libc::write(1, message.as_ptr() as *const libc::c_void, message.len());
            libc::_exit(130);
        }
    }
    unsafe {
        libc::signal(libc::SIGINT, handler as usize);
    }
}
