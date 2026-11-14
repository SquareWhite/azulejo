//! `azj`: command-line client for the Azulejo socket API.

fn usage() -> String {
    format!(
        "azj {} (API v{})\nsocket: {}",
        env!("CARGO_PKG_VERSION"),
        azulejo_protocol::API_VERSION,
        azulejo_api::socket_path().display()
    )
}

fn main() {
    println!("{}", usage());
}

#[cfg(test)]
mod tests {
    #[test]
    fn usage_names_the_api_version() {
        assert!(super::usage().contains("API v1"));
    }
}
