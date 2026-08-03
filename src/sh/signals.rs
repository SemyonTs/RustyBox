use crate::sh::globals::*;
use std::sync::atomic::Ordering;

extern "C" fn sigint_handler(_: libc::c_int) {
    SIGINT_RECEIVED.store(true, Ordering::SeqCst);
}

extern "C" fn sigtstp_handler(_: libc::c_int) {
    SIGTSTP_RECEIVED.store(true, Ordering::SeqCst);
}

pub fn setup_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigint_handler as *const () as usize;
        sa.sa_flags = 0;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());

        let mut sa_ignore: libc::sigaction = std::mem::zeroed();
        sa_ignore.sa_sigaction = libc::SIG_IGN as usize;
        sa_ignore.sa_flags = 0;
        libc::sigemptyset(&mut sa_ignore.sa_mask);
        libc::sigaction(libc::SIGQUIT, &sa_ignore, std::ptr::null_mut());

        let mut sa_tstp: libc::sigaction = std::mem::zeroed();
        sa_tstp.sa_sigaction = sigtstp_handler as *const () as usize;
        sa_tstp.sa_flags = 0;
        libc::sigemptyset(&mut sa_tstp.sa_mask);
        libc::sigaction(libc::SIGTSTP, &sa_tstp, std::ptr::null_mut());

        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }
}

pub fn setup_nproc_limit() {
    unsafe {
        let mut rlim: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NPROC, &mut rlim) == 0 {
            let desired: libc::rlim_t = 4096;
            if rlim.rlim_max == libc::RLIM_INFINITY || rlim.rlim_max > desired {
                rlim.rlim_cur = desired;
                libc::setrlimit(libc::RLIMIT_NPROC, &rlim);
            }
        }
    }
}

pub fn acquire_child_slot() -> bool {
    loop {
        let current = ACTIVE_CHILDREN.load(Ordering::SeqCst);
        if current >= MAX_CHILD_PROCESSES {
            return false;
        }
        if ACTIVE_CHILDREN
            .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return true;
        }
    }
}

pub fn release_child_slot() {
    loop {
        let current = ACTIVE_CHILDREN.load(Ordering::SeqCst);
        if current == 0 {
            return;
        }
        if ACTIVE_CHILDREN
            .compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
    }
}
