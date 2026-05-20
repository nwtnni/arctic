//! <https://pvk.ca/Blog/2020/07/07/flatter-wait-free-hazard-pointers/>
//! <https://docs.rs/membarrier/latest/membarrier/>

use core::sync::atomic::Ordering;

#[inline(always)]
pub(crate) fn fast_store_ordering(membarrier: bool) -> Ordering {
    if cfg!(feature = "opt-membarrier") && membarrier {
        Ordering::Relaxed
    } else {
        Ordering::SeqCst
    }
}

#[inline(always)]
pub(crate) fn fast_barrier(membarrier: bool) {
    if cfg!(feature = "opt-membarrier") && membarrier {
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
    }
}

#[cfg(feature = "opt-membarrier")]
pub(crate) fn slow(membarrier: bool) {
    if !membarrier {
        core::sync::atomic::fence(Ordering::SeqCst);
        return;
    }

    static INIT: std::sync::Once = std::sync::Once::new();

    INIT.call_once(|| {
        match unsafe {
            libc::syscall(
                libc::SYS_membarrier,
                libc::MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED,
                0,
                0,
            )
        } {
            0 => (),
            _ => panic!("membarrier register: {:?}", std::io::Error::last_os_error()),
        }
    });

    match unsafe {
        libc::syscall(
            libc::SYS_membarrier,
            libc::MEMBARRIER_CMD_PRIVATE_EXPEDITED,
            0,
            0,
        )
    } {
        0 => (),
        _ => panic!("membarrier: {:?}", std::io::Error::last_os_error()),
    }
}

#[cfg(not(feature = "opt-membarrier"))]
#[inline(always)]
pub(crate) fn slow(_membarrier: bool) {
    core::sync::atomic::fence(Ordering::SeqCst);
}
