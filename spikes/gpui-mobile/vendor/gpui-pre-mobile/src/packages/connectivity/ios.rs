use super::ConnectivityStatus;

#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {}

/// Check connectivity using SCNetworkReachability (available without extra frameworks).
pub fn check_connectivity() -> ConnectivityStatus {
    unsafe {
        // Create a reachability reference for 0.0.0.0 (general internet connectivity)
        let mut zero_addr: sockaddr_in = std::mem::zeroed();
        zero_addr.sin_len = std::mem::size_of::<sockaddr_in>() as u8;
        zero_addr.sin_family = 2; // AF_INET

        let reachability = SCNetworkReachabilityCreateWithAddress(
            std::ptr::null(),
            &zero_addr as *const sockaddr_in as *const sockaddr,
        );
        if reachability.is_null() {
            return ConnectivityStatus::None;
        }

        let mut flags: u32 = 0;
        let success = SCNetworkReachabilityGetFlags(reachability, &mut flags);
        CFRelease(reachability as *const _);

        if !success {
            return ConnectivityStatus::None;
        }

        let reachable = flags & SC_NETWORK_REACHABILITY_FLAGS_REACHABLE != 0;
        let needs_connection = flags & SC_NETWORK_REACHABILITY_FLAGS_CONNECTION_REQUIRED != 0;
        let is_wwan = flags & SC_NETWORK_REACHABILITY_FLAGS_IS_WWAN != 0;

        if !reachable || needs_connection {
            ConnectivityStatus::None
        } else if is_wwan {
            ConnectivityStatus::Cellular
        } else {
            ConnectivityStatus::Wifi
        }
    }
}

// SCNetworkReachability constants
const SC_NETWORK_REACHABILITY_FLAGS_REACHABLE: u32 = 1 << 1;
const SC_NETWORK_REACHABILITY_FLAGS_CONNECTION_REQUIRED: u32 = 1 << 2;
const SC_NETWORK_REACHABILITY_FLAGS_IS_WWAN: u32 = 1 << 18;

// Minimal C type definitions
#[repr(C)]
struct sockaddr {
    sa_len: u8,
    sa_family: u8,
    sa_data: [i8; 14],
}

#[repr(C)]
struct sockaddr_in {
    sin_len: u8,
    sin_family: u8,
    sin_port: u16,
    sin_addr: u32,
    sin_zero: [i8; 8],
}

// SCNetworkReachability is an opaque type
type SCNetworkReachabilityRef = *const std::ffi::c_void;

extern "C" {
    fn SCNetworkReachabilityCreateWithAddress(
        allocator: *const std::ffi::c_void,
        address: *const sockaddr,
    ) -> SCNetworkReachabilityRef;

    fn SCNetworkReachabilityGetFlags(target: SCNetworkReachabilityRef, flags: *mut u32) -> bool;

    fn CFRelease(cf: *const std::ffi::c_void);
}
