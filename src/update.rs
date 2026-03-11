/// Version comparison result.
#[derive(Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    AlreadyLatest(String),
    UpdateAvailable(String),
}

/// Compare semver versions. `latest_version` accepts both "v0.14.0" and "0.14.0".
pub fn check_update_needed(current: &str, latest: &str) -> UpdateStatus {
    let current_stripped = current.strip_prefix('v').unwrap_or(current);
    let latest_stripped = latest.strip_prefix('v').unwrap_or(latest);

    if current_stripped == latest_stripped {
        UpdateStatus::AlreadyLatest(latest.to_string())
    } else {
        UpdateStatus::UpdateAvailable(latest.to_string())
    }
}

/// Fetch the latest binary from GitHub Releases and replace the current executable.
pub fn perform_update() -> Result<self_update::Status, Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("maedana")
        .repo_name("crmux")
        .bin_name("crmux")
        .show_download_progress(true)
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()?
        .update()?;
    Ok(status)
}

/// Force update: always download the latest release regardless of current version.
pub fn perform_update_force() -> Result<self_update::Status, Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("maedana")
        .repo_name("crmux")
        .bin_name("crmux")
        .show_download_progress(true)
        .current_version("0.0.0")
        .build()?
        .update()?;
    Ok(status)
}

/// Fetch the latest version tag from GitHub Releases (check only, no update).
pub fn fetch_latest_version() -> Result<String, Box<dyn std::error::Error>> {
    let release = self_update::backends::github::Update::configure()
        .repo_owner("maedana")
        .repo_name("crmux")
        .bin_name("crmux")
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()?
        .get_latest_release()?;
    Ok(release.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_version() {
        assert_eq!(
            check_update_needed("0.13.0", "0.13.0"),
            UpdateStatus::AlreadyLatest("0.13.0".to_string())
        );
    }

    #[test]
    fn test_same_version_with_v_prefix() {
        assert_eq!(
            check_update_needed("0.13.0", "v0.13.0"),
            UpdateStatus::AlreadyLatest("v0.13.0".to_string())
        );
    }

    #[test]
    fn test_update_available() {
        assert_eq!(
            check_update_needed("0.13.0", "0.14.0"),
            UpdateStatus::UpdateAvailable("0.14.0".to_string())
        );
    }

    #[test]
    fn test_update_available_with_v_prefix() {
        assert_eq!(
            check_update_needed("0.13.0", "v0.14.0"),
            UpdateStatus::UpdateAvailable("v0.14.0".to_string())
        );
    }
}
