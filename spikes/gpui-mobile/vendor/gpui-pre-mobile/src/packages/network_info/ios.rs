use super::NetworkInfo;
use std::ffi::CStr;

#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {}

pub fn get_network_info() -> Result<NetworkInfo, String> {
    let mut info = NetworkInfo::default();

    // Get WiFi SSID/BSSID via CNCopyCurrentNetworkInfo
    unsafe {
        // Get list of supported interfaces
        let interfaces = CNCopySupportedInterfaces();
        if !interfaces.is_null() {
            let count = CFArrayGetCount(interfaces);
            for i in 0..count {
                let iface = CFArrayGetValueAtIndex(interfaces, i);
                let network_info = CNCopyCurrentNetworkInfo(iface as _);
                if !network_info.is_null() {
                    // Extract SSID
                    let ssid_key = cfstring_from_static(b"SSID\0");
                    let ssid_value = CFDictionaryGetValue(network_info, ssid_key as _);
                    if !ssid_value.is_null() {
                        info.wifi_name = cfstring_to_string(ssid_value as _);
                    }

                    // Extract BSSID
                    let bssid_key = cfstring_from_static(b"BSSID\0");
                    let bssid_value = CFDictionaryGetValue(network_info, bssid_key as _);
                    if !bssid_value.is_null() {
                        info.wifi_bssid = cfstring_to_string(bssid_value as _);
                    }

                    CFRelease(network_info as _);
                    break; // Use first interface
                }
            }
            CFRelease(interfaces as _);
        }
    }

    // Get IP address via getifaddrs
    info.wifi_ip = get_wifi_ip_address();

    Ok(info)
}

fn get_wifi_ip_address() -> Option<String> {
    unsafe {
        let mut ifaddr: *mut ifaddrs = std::ptr::null_mut();
        if getifaddrs(&mut ifaddr) != 0 {
            return None;
        }

        let mut result = None;
        let mut current = ifaddr;
        while !current.is_null() {
            let ifa = &*current;
            if !ifa.ifa_addr.is_null() {
                let family = (*ifa.ifa_addr).sa_family as i32;
                // AF_INET = 2 (IPv4)
                if family == 2 {
                    let name = CStr::from_ptr(ifa.ifa_name).to_string_lossy();
                    // en0 is typically WiFi on iOS
                    if name == "en0" {
                        let addr = ifa.ifa_addr as *const sockaddr_in;
                        let ip = (*addr).sin_addr;
                        let _bytes = ip.to_be_bytes();
                        // Network byte order to dotted-quad
                        let ip_str = format!(
                            "{}.{}.{}.{}",
                            ip & 0xFF,
                            (ip >> 8) & 0xFF,
                            (ip >> 16) & 0xFF,
                            (ip >> 24) & 0xFF,
                        );
                        result = Some(ip_str);
                        break;
                    }
                }
            }
            current = ifa.ifa_next;
        }

        freeifaddrs(ifaddr);
        result
    }
}

// CoreFoundation / SystemConfiguration types and functions

type CFAllocatorRef = *const std::ffi::c_void;
type CFArrayRef = *const std::ffi::c_void;
type CFDictionaryRef = *const std::ffi::c_void;
type CFStringRef = *const std::ffi::c_void;
type CFIndex = isize;

extern "C" {
    fn CNCopySupportedInterfaces() -> CFArrayRef;
    fn CNCopyCurrentNetworkInfo(interface_name: CFStringRef) -> CFDictionaryRef;
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const std::ffi::c_void;
    fn CFDictionaryGetValue(
        dict: CFDictionaryRef,
        key: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
    fn CFRelease(cf: *const std::ffi::c_void);
    fn CFStringGetCStringPtr(string: CFStringRef, encoding: u32) -> *const i8;
    fn CFStringGetLength(string: CFStringRef) -> CFIndex;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut i8,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> bool;
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const i8,
        encoding: u32,
    ) -> CFStringRef;
}

const CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

unsafe fn cfstring_from_static(s: &[u8]) -> CFStringRef {
    CFStringCreateWithCString(
        std::ptr::null(),
        s.as_ptr() as *const i8,
        CF_STRING_ENCODING_UTF8,
    )
}

unsafe fn cfstring_to_string(cf: CFStringRef) -> Option<String> {
    if cf.is_null() {
        return None;
    }
    // Try fast path
    let ptr = CFStringGetCStringPtr(cf, CF_STRING_ENCODING_UTF8);
    if !ptr.is_null() {
        return Some(CStr::from_ptr(ptr).to_string_lossy().into_owned());
    }
    // Slow path: copy to buffer
    let len = CFStringGetLength(cf);
    let buf_size = len * 4 + 1; // UTF-8 worst case
    let mut buf = vec![0i8; buf_size as usize];
    if CFStringGetCString(cf, buf.as_mut_ptr(), buf_size, CF_STRING_ENCODING_UTF8) {
        Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    } else {
        None
    }
}

// POSIX networking types

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

#[repr(C)]
struct ifaddrs {
    ifa_next: *mut ifaddrs,
    ifa_name: *const i8,
    ifa_flags: u32,
    ifa_addr: *const sockaddr,
    ifa_netmask: *const sockaddr,
    ifa_dstaddr: *const sockaddr,
    ifa_data: *mut std::ffi::c_void,
}

extern "C" {
    fn getifaddrs(ifap: *mut *mut ifaddrs) -> i32;
    fn freeifaddrs(ifa: *mut ifaddrs);
}
