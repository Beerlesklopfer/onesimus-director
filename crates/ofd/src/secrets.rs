//! Keeps passwords out of messages: plugin option strings
//! ("name:key=value:key=value", `\:` escapes a colon) carry credentials
//! (e.g. `password=` of the LDAP plugin), and the Job log is readable by
//! every console with access to the job.

const MASK: &str = "*****";

/// Substrings of an option name whose value is a secret.
const SECRET_KEYS: &[&str] = &["pass", "pwd", "secret", "token", "credential", "apikey", "api_key"];

fn is_secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEYS.iter().any(|s| key.contains(s))
}

/// `text` (a plugin option string, or a command line that holds one as its
/// last part) with the value of every secret `key=value` option replaced by
/// `*****`. Text without such an option is returned unchanged.
pub fn mask_plugin_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    // Start of the current ':'-separated option: its key is read up to '='.
    let mut key = String::new();
    let mut in_key = true;
    let mut masking = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let next = chars.next();
                if in_key {
                    key.push(c);
                    key.extend(next);
                }
                if !masking {
                    out.push(c);
                    out.extend(next);
                }
            }
            ':' => {
                masking = false;
                in_key = true;
                key.clear();
                out.push(c);
            }
            '=' if in_key => {
                in_key = false;
                out.push(c);
                if is_secret(&key) {
                    masking = true;
                    out.push_str(MASK);
                }
            }
            _ => {
                if in_key {
                    key.push(c);
                }
                if !masking {
                    out.push(c);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_of_the_ldap_plugin_is_masked() {
        let s = "python:module_name=bareos-fd-ldap:uri=ldaps\\://ldap.example.org:basedn=dc=x,dc=y:bind_dn=cn=admin,dc=x:password=not-a-real-secret";
        assert_eq!(mask_plugin_secrets(s),
            "python:module_name=bareos-fd-ldap:uri=ldaps\\://ldap.example.org:basedn=dc=x,dc=y:bind_dn=cn=admin,dc=x:password=*****");
    }

    #[test]
    fn other_options_stay_and_escaped_colons_belong_to_the_value() {
        assert_eq!(mask_plugin_secrets("p:Password=a\\:b:host=h:db_pass=x"), "p:Password=*****:host=h:db_pass=*****");
        assert_eq!(mask_plugin_secrets("postgresql:db=app:user=bob"), "postgresql:db=app:user=bob");
        assert_eq!(mask_plugin_secrets("no options"), "no options");
    }
}
