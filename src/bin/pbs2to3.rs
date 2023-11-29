use std::io::Write;
use std::path::Path;

use anyhow::{format_err, Error};
use regex::Regex;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use proxmox_apt::repositories::{self, APTRepositoryFile, APTRepositoryPackageType};
use proxmox_backup::api2::node::apt;

const OLD_SUITE: &str = "bullseye";
const NEW_SUITE: &str = "bookworm";
const PROXMOX_BACKUP_META: &str = "proxmox-backup";
const MIN_PBS_MAJOR: u8 = 2;
const MIN_PBS_MINOR: u8 = 4;
const MIN_PBS_PKGREL: u8 = 1;

fn main() -> Result<(), Error> {
    let mut checker = Checker::new();
    checker.check_pbs_packages()?;
    checker.check_misc()?;
    checker.summary()?;
    Ok(())
}

struct Checker {
    output: ConsoleOutput,
    upgraded: bool,
}

impl Checker {
    pub fn new() -> Self {
        Self {
            output: ConsoleOutput::new(),
            upgraded: false,
        }
    }

    pub fn check_pbs_packages(&mut self) -> Result<(), Error> {
        self.output
            .print_header("CHECKING VERSION INFORMATION FOR PBS PACKAGES")?;

        self.check_upgradable_packages()?;
        let pkg_versions = apt::get_versions()?;
        self.check_meta_package_version(&pkg_versions)?;
        self.check_kernel_compat(&pkg_versions)?;
        Ok(())
    }

    fn check_upgradable_packages(&mut self) -> Result<(), Error> {
        self.output.log_info("Checking for package updates..")?;

        let result = Self::get_upgradable_packages();
        match result {
            Err(err) => {
                self.output.log_warn(format!("{err}"))?;
                self.output
                    .log_fail("unable to retrieve list of package updates!")?;
            }
            Ok(cache) => {
                if cache.package_status.is_empty() {
                    self.output.log_pass("all packages up-to-date")?;
                } else {
                    let pkgs = cache
                        .package_status
                        .iter()
                        .map(|pkg| pkg.package.clone())
                        .collect::<Vec<String>>()
                        .join(", ");
                    self.output.log_warn(format!(
                        "updates for the following packages are available:\n      {pkgs}",
                    ))?;
                }
            }
        }
        Ok(())
    }

    fn check_meta_package_version(
        &mut self,
        pkg_versions: &[pbs_api_types::APTUpdateInfo],
    ) -> Result<(), Error> {
        self.output
            .log_info("Checking proxmox backup server package version..")?;

        let pbs_meta_pkg = pkg_versions
            .iter()
            .find(|pkg| pkg.package.as_str() == PROXMOX_BACKUP_META);

        if let Some(pbs_meta_pkg) = pbs_meta_pkg {
            let pkg_version = Regex::new(r"^(\d+)\.(\d+)[.-](\d+)")?;
            let captures = pkg_version.captures(&pbs_meta_pkg.old_version);
            if let Some(captures) = captures {
                let maj = Self::extract_version_from_captures(1, &captures)?;
                let min = Self::extract_version_from_captures(2, &captures)?;
                let pkgrel = Self::extract_version_from_captures(3, &captures)?;

                if maj > MIN_PBS_MAJOR {
                    self.output
                        .log_pass(format!("Already upgraded to Proxmox Backup Server {maj}"))?;
                    self.upgraded = true;
                } else if maj >= MIN_PBS_MAJOR && min >= MIN_PBS_MINOR && pkgrel >= MIN_PBS_PKGREL {
                    self.output.log_pass(format!(
                        "'{}' has version >= {}.{}-{}",
                        PROXMOX_BACKUP_META, MIN_PBS_MAJOR, MIN_PBS_MINOR, MIN_PBS_PKGREL,
                    ))?;
                } else {
                    self.output.log_fail(format!(
                        "'{}' package is too old, please upgrade to >= {}.{}-{}",
                        PROXMOX_BACKUP_META, MIN_PBS_MAJOR, MIN_PBS_MINOR, MIN_PBS_PKGREL,
                    ))?;
                }
            } else {
                self.output.log_fail(format!(
                    "could not match the '{PROXMOX_BACKUP_META}' package version, \
                    is it installed?",
                ))?;
            }
        } else {
            self.output
                .log_fail(format!("'{PROXMOX_BACKUP_META}' package not found!"))?;
        }
        Ok(())
    }

    fn check_kernel_compat(
        &mut self,
        pkg_versions: &[pbs_api_types::APTUpdateInfo],
    ) -> Result<(), Error> {
        self.output.log_info("Check running kernel version..")?;
        let (krunning, kinstalled) = if self.upgraded {
            (
                Regex::new(r"^6\.(?:2\.(?:[2-9]\d+|1[6-8]|1\d\d+)|5)[^~]*$")?,
                "proxmox-kernel-6.2",
            )
        } else {
            (Regex::new(r"^(?:5\.(?:13|15)|6\.2)")?, "pve-kernel-5.15")
        };

        let output = std::process::Command::new("uname").arg("-r").output();
        match output {
            Err(_err) => self
                .output
                .log_fail("unable to determine running kernel version.")?,
            Ok(ret) => {
                let running_version = std::str::from_utf8(&ret.stdout[..ret.stdout.len() - 1])?;
                if krunning.is_match(running_version) {
                    if self.upgraded {
                        self.output.log_pass(format!(
                            "running new kernel '{running_version}' after upgrade."
                        ))?;
                    } else {
                        self.output.log_pass(format!(
                            "running kernel '{running_version}' is considered suitable for \
                            upgrade."
                        ))?;
                    }
                } else {
                    let installed_kernel = pkg_versions
                        .iter()
                        .find(|pkg| pkg.package.as_str() == kinstalled);
                    if installed_kernel.is_some() {
                        self.output.log_warn(format!(
                            "a suitable kernel '{kinstalled}' is installed, but an \
                            unsuitable '{running_version}' is booted, missing reboot?!",
                        ))?;
                    } else {
                        self.output.log_warn(format!(
                            "unexpected running and installed kernel '{running_version}'.",
                        ))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn extract_version_from_captures(
        index: usize,
        captures: &regex::Captures,
    ) -> Result<u8, Error> {
        if let Some(capture) = captures.get(index) {
            let val = capture.as_str().parse::<u8>()?;
            Ok(val)
        } else {
            Ok(0)
        }
    }

    fn check_bootloader(&mut self) -> Result<(), Error> {
        self.output
            .log_info("Checking bootloader configuration...")?;

        if !Path::new("/sys/firmware/efi").is_dir() {
            self.output
                .log_skip("System booted in legacy-mode - no need for systemd-boot")?;
            return Ok(());
        }

        if Path::new("/etc/kernel/proxmox-boot-uuids").is_file() {
            // PBS packages version check needs to be run before
            if !self.upgraded {
                self.output
                    .log_skip("not yet upgraded, no need to check the presence of systemd-boot")?;
                return Ok(());
            }
            if Path::new("/usr/share/doc/systemd-boot/changelog.Debian.gz").is_file() {
                self.output
                    .log_pass("bootloader packages installed correctly")?;
                return Ok(());
            }
            self.output.log_warn(
                "proxmox-boot-tool is used for bootloader configuration in uefi mode \
                 but the separate systemd-boot package, is not installed.\n\
                 initializing new ESPs will not work until the package is installed.",
            )?;
            return Ok(());
        } else if !Path::new("/usr/share/doc/grub-efi-amd64/changelog.Debian.gz").is_file() {
            self.output.log_warn(
                "System booted in uefi mode but grub-efi-amd64 meta-package not installed, \
             new grub versions will not be installed to /boot/efi!
             Install grub-efi-amd64.",
            )?;
            return Ok(());
        } else {
            self.output
                .log_pass("bootloader packages installed correctly")?;
        }

        Ok(())
    }

    fn check_apt_repos(&mut self) -> Result<(), Error> {
        self.output
            .log_info("Checking for package repository suite mismatches..")?;

        let mut strange_suite = false;
        let mut mismatches = Vec::new();
        let mut found_suite: Option<(String, String)> = None;

        let (repo_files, _repo_errors, _digest) = repositories::repositories()?;
        for repo_file in repo_files {
            self.check_repo_file(
                &mut found_suite,
                &mut mismatches,
                &mut strange_suite,
                repo_file,
            )?;
        }

        match (mismatches.is_empty(), strange_suite) {
            (true, false) => self.output.log_pass("found no suite mismatch")?,
            (true, true) => self
                .output
                .log_notice("found no suite mismatches, but found at least one strange suite")?,
            (false, _) => {
                let mut message = String::from(
                    "Found mixed old and new packages repository suites, fix before upgrading!\
                    \n      Mismatches:",
                );
                for (suite, location) in mismatches.iter() {
                    message.push_str(
                        format!("\n      found suite '{suite}' at '{location}'").as_str(),
                    );
                }
                message.push('\n');
                self.output.log_fail(message)?
            }
        }

        Ok(())
    }

    fn check_dkms_modules(&mut self) -> Result<(), Error> {
        let kver = std::process::Command::new("uname")
            .arg("-r")
            .output()
            .map_err(|err| format_err!("failed to retrieve running kernel version - {err}"))?;

        let output = std::process::Command::new("dkms")
            .arg("status")
            .arg("-k")
            .arg(std::str::from_utf8(&kver.stdout)?)
            .output();
        match output {
            Err(_err) => self.output.log_skip("could not get dkms status")?,
            Ok(ret) => {
                let num_dkms_modules = std::str::from_utf8(&ret.stdout)?.lines().count();
                if num_dkms_modules == 0 {
                    self.output.log_pass("no dkms modules found")?;
                } else {
                    self.output
                        .log_warn("dkms modules found, this might cause issues during upgrade.")?;
                }
            }
        }
        Ok(())
    }

    pub fn check_misc(&mut self) -> Result<(), Error> {
        self.output.print_header("MISCELLANEOUS CHECKS")?;
        self.check_pbs_services()?;
        self.check_time_sync()?;
        self.check_apt_repos()?;
        self.check_bootloader()?;
        self.check_dkms_modules()?;
        Ok(())
    }

    pub fn summary(&mut self) -> Result<(), Error> {
        self.output.print_summary()
    }

    fn check_repo_file(
        &mut self,
        found_suite: &mut Option<(String, String)>,
        mismatches: &mut Vec<(String, String)>,
        strange_suite: &mut bool,
        repo_file: APTRepositoryFile,
    ) -> Result<(), Error> {
        for repo in repo_file.repositories {
            if !repo.enabled || repo.types == [APTRepositoryPackageType::DebSrc] {
                continue;
            }
            for suite in &repo.suites {
                let suite = match suite.find(&['-', '/'][..]) {
                    Some(n) => &suite[0..n],
                    None => suite,
                };

                if suite != OLD_SUITE && suite != NEW_SUITE {
                    let location = repo_file.path.clone().unwrap_or_default();
                    self.output.log_notice(format!(
                        "found unusual suite '{suite}', neither old '{OLD_SUITE}' nor new \
                            '{NEW_SUITE}'..\n        Affected file {location}\n        Please \
                            assure this is shipping compatible packages for the upgrade!"
                    ))?;
                    *strange_suite = true;
                    continue;
                }

                if let Some((ref current_suite, ref current_location)) = found_suite {
                    let location = repo_file.path.clone().unwrap_or_default();
                    if suite != current_suite {
                        if mismatches.is_empty() {
                            mismatches.push((current_suite.clone(), current_location.clone()));
                            mismatches.push((suite.to_string(), location));
                        } else {
                            mismatches.push((suite.to_string(), location));
                        }
                    }
                } else {
                    let location = repo_file.path.clone().unwrap_or_default();
                    *found_suite = Some((suite.to_string(), location));
                }
            }
        }
        Ok(())
    }

    fn get_systemd_unit_state(
        &mut self,
        unit: &str,
    ) -> Result<(SystemdUnitState, SystemdUnitState), Error> {
        let output = std::process::Command::new("systemctl")
            .arg("is-enabled")
            .arg(unit)
            .output()
            .map_err(|err| format_err!("failed to execute - {err}"))?;

        let enabled_state = match output.stdout.as_slice() {
            b"enabled\n" => SystemdUnitState::Enabled,
            b"disabled\n" => SystemdUnitState::Disabled,
            _ => SystemdUnitState::Unknown,
        };

        let output = std::process::Command::new("systemctl")
            .arg("is-active")
            .arg(unit)
            .output()
            .map_err(|err| format_err!("failed to execute - {err}"))?;

        let active_state = match output.stdout.as_slice() {
            b"active\n" => SystemdUnitState::Active,
            b"inactive\n" => SystemdUnitState::Inactive,
            b"failed\n" => SystemdUnitState::Failed,
            _ => SystemdUnitState::Unknown,
        };
        Ok((enabled_state, active_state))
    }

    fn check_pbs_services(&mut self) -> Result<(), Error> {
        self.output.log_info("Checking PBS daemon services..")?;

        for service in ["proxmox-backup.service", "proxmox-backup-proxy.service"] {
            match self.get_systemd_unit_state(service)? {
                (_, SystemdUnitState::Active) => {
                    self.output
                        .log_pass(format!("systemd unit '{service}' is in state 'active'"))?;
                }
                (_, SystemdUnitState::Inactive) => {
                    self.output.log_fail(format!(
                        "systemd unit '{service}' is in state 'inactive'\
                            \n    Please check the service for errors and start it.",
                    ))?;
                }
                (_, SystemdUnitState::Failed) => {
                    self.output.log_fail(format!(
                        "systemd unit '{service}' is in state 'failed'\
                            \n    Please check the service for errors and start it.",
                    ))?;
                }
                (_, _) => {
                    self.output.log_fail(format!(
                        "systemd unit '{service}' is not in state 'active'\
                            \n    Please check the service for errors and start it.",
                    ))?;
                }
            }
        }
        Ok(())
    }

    fn check_time_sync(&mut self) -> Result<(), Error> {
        self.output
            .log_info("Checking for supported & active NTP service..")?;
        if self.get_systemd_unit_state("systemd-timesyncd.service")?.1 == SystemdUnitState::Active {
            self.output.log_warn(
                "systemd-timesyncd is not the best choice for time-keeping on servers, due to only \
                applying updates on boot.\
                \n       While not necessary for the upgrade it's recommended to use one of:\
                \n        * chrony (Default in new Proxmox Backup Server installations)\
                \n        * ntpsec\
                \n        * openntpd"
            )?;
        } else if self.get_systemd_unit_state("ntp.service")?.1 == SystemdUnitState::Active {
            self.output.log_info(
                "Debian deprecated and removed the ntp package for Bookworm, but the system \
                    will automatically migrate to the 'ntpsec' replacement package on upgrade.",
            )?;
        } else if self.get_systemd_unit_state("chrony.service")?.1 == SystemdUnitState::Active
            || self.get_systemd_unit_state("openntpd.service")?.1 == SystemdUnitState::Active
            || self.get_systemd_unit_state("ntpsec.service")?.1 == SystemdUnitState::Active
        {
            self.output
                .log_pass("Detected active time synchronisation unit")?;
        } else {
            self.output.log_warn(
                "No (active) time synchronisation daemon (NTP) detected, but synchronized systems \
                are important!",
            )?;
        }
        Ok(())
    }

    fn get_upgradable_packages() -> Result<proxmox_backup::tools::apt::PkgState, Error> {
        let cache = if let Ok(false) = proxmox_backup::tools::apt::pkg_cache_expired() {
            if let Ok(Some(cache)) = proxmox_backup::tools::apt::read_pkg_state() {
                cache
            } else {
                proxmox_backup::tools::apt::update_cache()?
            }
        } else {
            proxmox_backup::tools::apt::update_cache()?
        };

        Ok(cache)
    }
}

#[derive(PartialEq)]
enum SystemdUnitState {
    Active,
    Enabled,
    Disabled,
    Failed,
    Inactive,
    Unknown,
}

#[derive(Default)]
struct Counters {
    pass: u64,
    skip: u64,
    notice: u64,
    warn: u64,
    fail: u64,
}

enum LogLevel {
    Pass,
    Info,
    Skip,
    Notice,
    Warn,
    Fail,
}

struct ConsoleOutput {
    stream: StandardStream,
    first_header: bool,
    counters: Counters,
}

impl ConsoleOutput {
    pub fn new() -> Self {
        Self {
            stream: StandardStream::stdout(ColorChoice::Always),
            first_header: true,
            counters: Counters::default(),
        }
    }

    pub fn print_header(&mut self, message: &str) -> Result<(), Error> {
        if !self.first_header {
            writeln!(&mut self.stream)?;
        }
        self.first_header = false;
        writeln!(&mut self.stream, "= {message} =\n")?;
        Ok(())
    }

    pub fn set_color(&mut self, color: Color, bold: bool) -> Result<(), Error> {
        self.stream
            .set_color(ColorSpec::new().set_fg(Some(color)).set_bold(bold))?;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), std::io::Error> {
        self.stream.reset()
    }

    pub fn log_line(&mut self, level: LogLevel, message: &str) -> Result<(), Error> {
        match level {
            LogLevel::Pass => {
                self.counters.pass += 1;
                self.set_color(Color::Green, false)?;
                writeln!(&mut self.stream, "PASS: {}", message)?;
            }
            LogLevel::Info => {
                writeln!(&mut self.stream, "INFO: {}", message)?;
            }
            LogLevel::Skip => {
                self.counters.skip += 1;
                writeln!(&mut self.stream, "SKIP: {}", message)?;
            }
            LogLevel::Notice => {
                self.counters.notice += 1;
                self.set_color(Color::White, true)?;
                writeln!(&mut self.stream, "NOTICE: {}", message)?;
            }
            LogLevel::Warn => {
                self.counters.warn += 1;
                self.set_color(Color::Yellow, false)?;
                writeln!(&mut self.stream, "WARN: {}", message)?;
            }
            LogLevel::Fail => {
                self.counters.fail += 1;
                self.set_color(Color::Red, true)?;
                writeln!(&mut self.stream, "FAIL: {}", message)?;
            }
        }
        self.reset()?;
        Ok(())
    }

    pub fn log_pass<T: AsRef<str>>(&mut self, message: T) -> Result<(), Error> {
        self.log_line(LogLevel::Pass, message.as_ref())
    }

    pub fn log_info<T: AsRef<str>>(&mut self, message: T) -> Result<(), Error> {
        self.log_line(LogLevel::Info, message.as_ref())
    }

    pub fn log_skip<T: AsRef<str>>(&mut self, message: T) -> Result<(), Error> {
        self.log_line(LogLevel::Skip, message.as_ref())
    }

    pub fn log_notice<T: AsRef<str>>(&mut self, message: T) -> Result<(), Error> {
        self.log_line(LogLevel::Notice, message.as_ref())
    }

    pub fn log_warn<T: AsRef<str>>(&mut self, message: T) -> Result<(), Error> {
        self.log_line(LogLevel::Warn, message.as_ref())
    }

    pub fn log_fail<T: AsRef<str>>(&mut self, message: T) -> Result<(), Error> {
        self.log_line(LogLevel::Fail, message.as_ref())
    }

    pub fn print_summary(&mut self) -> Result<(), Error> {
        self.print_header("SUMMARY")?;

        let total = self.counters.fail
            + self.counters.pass
            + self.counters.notice
            + self.counters.skip
            + self.counters.warn;

        writeln!(&mut self.stream, "TOTAL:     {total}")?;
        self.set_color(Color::Green, false)?;
        writeln!(&mut self.stream, "PASSED:    {}", self.counters.pass)?;
        self.reset()?;
        writeln!(&mut self.stream, "SKIPPED:   {}", self.counters.skip)?;
        writeln!(&mut self.stream, "NOTICE:    {}", self.counters.notice)?;
        if self.counters.warn > 0 {
            self.set_color(Color::Yellow, false)?;
            writeln!(&mut self.stream, "WARNINGS:  {}", self.counters.warn)?;
        }
        if self.counters.fail > 0 {
            self.set_color(Color::Red, true)?;
            writeln!(&mut self.stream, "FAILURES:  {}", self.counters.fail)?;
        }
        if self.counters.warn > 0 || self.counters.fail > 0 {
            let (color, bold) = if self.counters.fail > 0 {
                (Color::Red, true)
            } else {
                (Color::Yellow, false)
            };

            self.set_color(color, bold)?;
            writeln!(
                &mut self.stream,
                "\nATTENTION: Please check the output for detailed information!",
            )?;
            if self.counters.fail > 0 {
                writeln!(
                    &mut self.stream,
                    "Try to solve the problems one at a time and rerun this checklist tool again.",
                )?;
            }
        }
        self.reset()?;
        Ok(())
    }
}
