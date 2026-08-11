use std::path::Path;

pub fn ssh_string(key: Option<&Path>) -> String {
    format!(
        "ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10{}",
        key.map(|k| format!(" -i {}", k.display()))
            .unwrap_or_default()
    )
}
