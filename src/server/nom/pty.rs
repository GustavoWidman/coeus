//! a pseudo-terminal pair, opened through `/dev/ptmx` with the platform's
//! own requests. rustix's `pty` feature would do the same, but enabling it
//! changes the build of every binary in the package, the runner launcher
//! included, whose bytes must not change

use std::{io, os::fd::OwnedFd};

use rustix::{
    fs::{Mode, OFlags},
    ioctl::{Getter, Opcode, ioctl, opcode},
};

pub(super) struct Pty {
    /// the side the daemon reads and resizes
    pub master: OwnedFd,
    /// the side nom draws on
    pub terminal: OwnedFd,
}

impl Pty {
    const FLAGS: OFlags = OFlags::RDWR.union(OFlags::NOCTTY).union(OFlags::CLOEXEC);

    pub fn open() -> io::Result<Self> {
        let master = rustix::fs::open("/dev/ptmx", Self::FLAGS, Mode::empty())?;
        let name = Self::unlock(&master)?;
        let terminal = rustix::fs::open(name, Self::FLAGS, Mode::empty())?;
        Ok(Self { master, terminal })
    }

    /// grants and unlocks the terminal of `master`, returning its path
    #[cfg(target_os = "linux")]
    fn unlock(master: &OwnedFd) -> io::Result<String> {
        use std::ffi::c_int;

        use rustix::ioctl::Setter;

        const TIOCSPTLCK: Opcode = opcode::write::<c_int>(b'T', 0x31);
        const TIOCGPTN: Opcode = opcode::read::<u32>(b'T', 0x30);

        // SAFETY: the requests and their argument types are Linux's
        let number = unsafe {
            ioctl(master, Setter::<TIOCSPTLCK, c_int>::new(0))?;
            ioctl(master, Getter::<TIOCGPTN, u32>::new())?
        };
        Ok(format!("/dev/pts/{number}"))
    }

    /// grants and unlocks the terminal of `master`, returning its path
    #[cfg(target_os = "macos")]
    fn unlock(master: &OwnedFd) -> io::Result<String> {
        use rustix::ioctl::NoArg;

        const TIOCPTYUNLK: Opcode = opcode::none(b't', 82);
        const TIOCPTYGNAME: Opcode = opcode::read::<[u8; 128]>(b't', 83);
        const TIOCPTYGRANT: Opcode = opcode::none(b't', 84);

        // SAFETY: the requests and their argument types are macOS's
        let name = unsafe {
            ioctl(master, NoArg::<TIOCPTYGRANT>::new())?;
            ioctl(master, NoArg::<TIOCPTYUNLK>::new())?;
            ioctl(master, Getter::<TIOCPTYGNAME, [u8; 128]>::new())?
        };
        let length = name
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(name.len());
        Ok(String::from_utf8_lossy(&name[..length]).into_owned())
    }
}
