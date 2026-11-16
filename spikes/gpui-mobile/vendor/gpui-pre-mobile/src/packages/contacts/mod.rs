//! Contacts access for reading the device address book.
//!
//! Provides a cross-platform contacts API backed by:
//! - Android: ContactsContract via JNI
//! - iOS: CNContactStore via Objective-C
//!
//! Feature-gated behind `contacts`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// A phone number entry.
#[derive(Debug, Clone)]
pub struct PhoneNumber {
    pub number: String,
    /// Label such as "mobile", "home", "work".
    pub label: String,
}

/// An email entry.
#[derive(Debug, Clone)]
pub struct EmailAddress {
    pub address: String,
    /// Label such as "home", "work".
    pub label: String,
}

/// A contact from the device address book.
#[derive(Debug, Clone)]
pub struct Contact {
    /// Platform-specific contact identifier.
    pub id: String,
    /// Display name.
    pub display_name: String,
    /// Given (first) name.
    pub given_name: String,
    /// Family (last) name.
    pub family_name: String,
    /// Phone numbers.
    pub phones: Vec<PhoneNumber>,
    /// Email addresses.
    pub emails: Vec<EmailAddress>,
}

/// Get all contacts from the device address book.
///
/// Requires contacts permission (use `permission_handler` to request first).
pub fn get_contacts() -> Result<Vec<Contact>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_contacts()
    }
    #[cfg(target_os = "android")]
    {
        android::get_contacts()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Ok(Vec::new())
    }
}

/// Search contacts by name query.
///
/// Requires contacts permission.
pub fn search_contacts(query: &str) -> Result<Vec<Contact>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::search_contacts(query)
    }
    #[cfg(target_os = "android")]
    {
        android::search_contacts(query)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = query;
        Ok(Vec::new())
    }
}

/// Get a single contact by ID.
///
/// Requires contacts permission.
pub fn get_contact(id: &str) -> Result<Option<Contact>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_contact(id)
    }
    #[cfg(target_os = "android")]
    {
        android::get_contact(id)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = id;
        Ok(None)
    }
}
