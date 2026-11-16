use super::{Contact, EmailAddress, PhoneNumber};
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};

pub fn get_contacts() -> Result<Vec<Contact>, String> {
    unsafe { fetch_contacts(std::ptr::null_mut()) }
}

pub fn search_contacts(query: &str) -> Result<Vec<Contact>, String> {
    unsafe {
        // CNContact.predicateForContactsMatchingName:
        let ns_query = new_nsstring(query);
        if ns_query.is_null() {
            return Err("Failed to create NSString for query".into());
        }
        let predicate: *mut AnyObject =
            msg_send![class!(CNContact), predicateForContactsMatchingName: ns_query];
        if predicate.is_null() {
            return Err("Failed to create search predicate".into());
        }
        fetch_contacts(predicate)
    }
}

pub fn get_contact(id: &str) -> Result<Option<Contact>, String> {
    unsafe {
        let ns_id = new_nsstring(id);
        if ns_id.is_null() {
            return Err("Failed to create NSString for id".into());
        }

        // Create an NSArray with the single identifier
        let ids_array: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: ns_id];
        if ids_array.is_null() {
            return Err("Failed to create NSArray for identifiers".into());
        }

        // CNContact.predicateForContactsWithIdentifiers:
        let predicate: *mut AnyObject =
            msg_send![class!(CNContact), predicateForContactsWithIdentifiers: ids_array];
        if predicate.is_null() {
            return Err("Failed to create identifier predicate".into());
        }

        let contacts = fetch_contacts(predicate)?;
        Ok(contacts.into_iter().next())
    }
}

/// Fetch contacts from CNContactStore, optionally filtered by a predicate.
///
/// If `predicate` is null, all contacts are returned.
unsafe fn fetch_contacts(predicate: *mut AnyObject) -> Result<Vec<Contact>, String> {
    let store: *mut AnyObject = msg_send![class!(CNContactStore), alloc];
    let store: *mut AnyObject = msg_send![store, init];
    if store.is_null() {
        return Err("Failed to create CNContactStore".into());
    }

    // Build the keys to fetch
    let keys = create_fetch_keys();
    if keys.is_null() {
        let _: () = msg_send![store, release];
        return Err("Failed to create fetch keys array".into());
    }

    // Create CNContactFetchRequest
    let request: *mut AnyObject = msg_send![class!(CNContactFetchRequest), alloc];
    let request: *mut AnyObject = msg_send![request, initWithKeysToFetch: keys];
    if request.is_null() {
        let _: () = msg_send![store, release];
        return Err("Failed to create CNContactFetchRequest".into());
    }

    // Set predicate if provided
    if !predicate.is_null() {
        let _: () = msg_send![request, setPredicate: predicate];
    }

    // Sort by given name
    let _: () = msg_send![request, setSortOrder: 1i64]; // CNContactSortOrderGivenName = 1

    // Enumerate contacts
    let contacts_array: *mut AnyObject = msg_send![class!(NSMutableArray), new];
    if contacts_array.is_null() {
        let _: () = msg_send![request, release];
        let _: () = msg_send![store, release];
        return Err("Failed to create results array".into());
    }

    // We use unifiedContactsMatchingPredicate:keysToFetch:error: for predicate-based,
    // or enumerateContactsWithFetchRequest:error:usingBlock: for all contacts.
    // The enumerate approach with blocks is complex in Rust. Instead, when no predicate
    // is supplied we use a "match everything" approach via enumerating with the fetch request.
    // Actually, unifiedContactsMatchingPredicate requires a non-nil predicate.
    // For the "all contacts" case, we use a different approach.

    let mut contacts = Vec::new();

    if predicate.is_null() {
        // For all contacts, we need to use enumerateContactsWithFetchRequest:error:usingBlock:
        // This requires a block. We'll use the block crate.
        let error: *mut AnyObject = std::ptr::null_mut();
        let error_ptr: *mut *mut AnyObject = &error as *const _ as *mut *mut AnyObject;

        // Use a simpler approach: fetch contacts matching an empty name predicate
        // Actually CNContact doesn't have such a predicate. Let's use the block-based enumerate.
        let success: bool = enumerate_contacts(store, request, error_ptr, &mut contacts);

        if !success && !error.is_null() {
            let desc: *mut AnyObject = msg_send![error, localizedDescription];
            let err_msg = nsstring_to_string(desc);
            let _: () = msg_send![contacts_array, release];
            let _: () = msg_send![request, release];
            let _: () = msg_send![store, release];
            return Err(format!("Failed to enumerate contacts: {}", err_msg));
        }
    } else {
        // Use unifiedContactsMatchingPredicate:keysToFetch:error:
        let error: *mut AnyObject = std::ptr::null_mut();
        let error_ptr: *mut *mut AnyObject = &error as *const _ as *mut *mut AnyObject;

        let results: *mut AnyObject = msg_send![store,
            unifiedContactsMatchingPredicate: predicate,
            keysToFetch: keys,
            error: error_ptr
        ];

        if results.is_null() {
            let err_msg = if !error.is_null() {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                nsstring_to_string(desc)
            } else {
                "Unknown error".to_owned()
            };
            let _: () = msg_send![contacts_array, release];
            let _: () = msg_send![request, release];
            let _: () = msg_send![store, release];
            return Err(format!("Failed to fetch contacts: {}", err_msg));
        }

        let count: usize = msg_send![results, count];
        for i in 0..count {
            let cn_contact: *mut AnyObject = msg_send![results, objectAtIndex: i];
            if let Some(contact) = parse_cn_contact(cn_contact) {
                contacts.push(contact);
            }
        }
    }

    let _: () = msg_send![contacts_array, release];
    let _: () = msg_send![request, release];
    let _: () = msg_send![store, release];

    Ok(contacts)
}

/// Enumerate all contacts using a fetch request and a block callback.
unsafe fn enumerate_contacts(
    store: *mut AnyObject,
    request: *mut AnyObject,
    error_ptr: *mut *mut AnyObject,
    contacts: &mut Vec<Contact>,
) -> bool {
    // We use block2 to create an Objective-C block for the enumeration callback.
    // The block signature is: void (^)(CNContact *contact, BOOL *stop)
    let contacts_ptr = contacts as *mut Vec<Contact>;

    let block = block2::RcBlock::new(move |cn_contact: *mut AnyObject, _stop: *mut Bool| {
        if !cn_contact.is_null() {
            if let Some(contact) = unsafe { parse_cn_contact(cn_contact) } {
                unsafe { (*contacts_ptr).push(contact) };
            }
        }
    });

    let success: bool = msg_send![store,
        enumerateContactsWithFetchRequest: request,
        error: error_ptr,
        usingBlock: &*block
    ];

    success
}

/// Parse a CNContact object into our Contact struct.
unsafe fn parse_cn_contact(cn_contact: *mut AnyObject) -> Option<Contact> {
    if cn_contact.is_null() {
        return None;
    }

    let identifier: *mut AnyObject = msg_send![cn_contact, identifier];
    let id = nsstring_to_string(identifier);

    let given_name_ns: *mut AnyObject = msg_send![cn_contact, givenName];
    let given_name = nsstring_to_string(given_name_ns);

    let family_name_ns: *mut AnyObject = msg_send![cn_contact, familyName];
    let family_name = nsstring_to_string(family_name_ns);

    // Build display name from given + family
    let display_name = if given_name.is_empty() && family_name.is_empty() {
        String::new()
    } else if family_name.is_empty() {
        given_name.clone()
    } else if given_name.is_empty() {
        family_name.clone()
    } else {
        format!("{} {}", given_name, family_name)
    };

    // Phone numbers: NSArray<CNLabeledValue<CNPhoneNumber *> *>
    let phone_numbers: *mut AnyObject = msg_send![cn_contact, phoneNumbers];
    let phones = parse_phone_numbers(phone_numbers);

    // Email addresses: NSArray<CNLabeledValue<NSString *> *>
    let email_addresses: *mut AnyObject = msg_send![cn_contact, emailAddresses];
    let emails = parse_email_addresses(email_addresses);

    Some(Contact {
        id,
        display_name,
        given_name,
        family_name,
        phones,
        emails,
    })
}

/// Parse an NSArray of CNLabeledValue<CNPhoneNumber> into phone number entries.
unsafe fn parse_phone_numbers(array: *mut AnyObject) -> Vec<PhoneNumber> {
    if array.is_null() {
        return Vec::new();
    }

    let count: usize = msg_send![array, count];
    let mut phones = Vec::with_capacity(count);

    for i in 0..count {
        let labeled_value: *mut AnyObject = msg_send![array, objectAtIndex: i];
        if labeled_value.is_null() {
            continue;
        }

        // Get the CNPhoneNumber value
        let phone_number_obj: *mut AnyObject = msg_send![labeled_value, value];
        if phone_number_obj.is_null() {
            continue;
        }

        let number_ns: *mut AnyObject = msg_send![phone_number_obj, stringValue];
        let number = nsstring_to_string(number_ns);

        // Get label
        let label_ns: *mut AnyObject = msg_send![labeled_value, label];
        let label = if label_ns.is_null() {
            "other".to_owned()
        } else {
            // Localize the label
            let localized: *mut AnyObject =
                msg_send![class!(CNLabeledValue), localizedStringForLabel: label_ns];
            if localized.is_null() {
                nsstring_to_string(label_ns)
            } else {
                nsstring_to_string(localized)
            }
        };

        phones.push(PhoneNumber { number, label });
    }

    phones
}

/// Parse an NSArray of CNLabeledValue<NSString> into email address entries.
unsafe fn parse_email_addresses(array: *mut AnyObject) -> Vec<EmailAddress> {
    if array.is_null() {
        return Vec::new();
    }

    let count: usize = msg_send![array, count];
    let mut emails = Vec::with_capacity(count);

    for i in 0..count {
        let labeled_value: *mut AnyObject = msg_send![array, objectAtIndex: i];
        if labeled_value.is_null() {
            continue;
        }

        let address_ns: *mut AnyObject = msg_send![labeled_value, value];
        let address = nsstring_to_string(address_ns);

        let label_ns: *mut AnyObject = msg_send![labeled_value, label];
        let label = if label_ns.is_null() {
            "other".to_owned()
        } else {
            let localized: *mut AnyObject =
                msg_send![class!(CNLabeledValue), localizedStringForLabel: label_ns];
            if localized.is_null() {
                nsstring_to_string(label_ns)
            } else {
                nsstring_to_string(localized)
            }
        };

        emails.push(EmailAddress { address, label });
    }

    emails
}

/// Create the NSArray of CNContact keys to fetch.
unsafe fn create_fetch_keys() -> *mut AnyObject {
    // We need CNContactIdentifierKey, CNContactGivenNameKey, CNContactFamilyNameKey,
    // CNContactPhoneNumbersKey, CNContactEmailAddressesKey
    let key_identifier = new_nsstring("identifier");
    let key_given_name = new_nsstring("givenName");
    let key_family_name = new_nsstring("familyName");
    let key_phone_numbers = new_nsstring("phoneNumbers");
    let key_email_addresses = new_nsstring("emailAddresses");

    let objects = [
        key_identifier,
        key_given_name,
        key_family_name,
        key_phone_numbers,
        key_email_addresses,
    ];

    let array: *mut AnyObject = msg_send![class!(NSArray),
        arrayWithObjects: objects.as_ptr(),
        count: objects.len()
    ];

    array
}

/// Create an NSString from a Rust &str.
unsafe fn new_nsstring(s: &str) -> *mut AnyObject {
    let ns: *mut AnyObject = msg_send![class!(NSString), alloc];
    let ns: *mut AnyObject = msg_send![ns,
        initWithBytes: s.as_ptr() as *const std::ffi::c_void,
        length: s.len(),
        encoding: 4u64  // NSUTF8StringEncoding
    ];
    ns
}

/// Convert an NSString to a Rust String.
unsafe fn nsstring_to_string(ns: *mut AnyObject) -> String {
    if ns.is_null() {
        return String::new();
    }
    let utf8: *const std::ffi::c_char = msg_send![ns, UTF8String];
    if utf8.is_null() {
        return String::new();
    }
    let c_str = std::ffi::CStr::from_ptr(utf8);
    match c_str.to_str() {
        Ok(s) => s.to_owned(),
        Err(_) => c_str.to_string_lossy().into_owned(),
    }
}
