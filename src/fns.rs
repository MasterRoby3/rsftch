use libmacchina::{traits::MemoryReadout as _, MemoryReadout};
use rayon::prelude::*;
use std::{
    env,
    fs::{self, read_to_string, File},
    io::{BufRead, BufReader, Error, Read},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

pub fn help() {
    println!(
        r#"Usage: rsftch [OPTION...] [OVERRIDE] [MARGIN] [CONFIG FILE(s)] [INFO]
        
      -h, --help, --usage         Bring up this menu.
      -v, --version               Print version number.
      -o, --override              Overrides distribution, affects ASCII and "distro" info. Running without
                                  an argument prints all possible options.
      -m, --margin                Add margin to the info sections, default 1.
      -c, --color-config          Specify another color config file, to be used instead of the default one.
      -i, --info-config           Specify another info config file, to be used instead of the default one.
          --ignore-color-config   Ignores the custom color config and uses the default one.
          --ignore-info-config    Ignores the custom info config and uses the default one.
          --ignore-config         Ignores both configs and uses the default ones.
          --info                  Only prints the value of the following arguments info, for example
                                  `rsftch --info distro` would output: "EndeavourOS".

    Info config is located at:  ~/.config/rsftch/info.json
    Color config is located at: ~/.config/rsftch/colors.json"#
    );
}

pub fn whoami() -> String {
    let output = Command::new("whoami")
        .output()
        .expect("`whoami` failed, are you on a Unix-like operating system?");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn home_dir() -> &'static str {
    if let Some(home) = option_env!("HOME") {
        return home;
    } else {
        eprintln!("Couldn't find home directory, are you on a Unix-like operating system? \nIf you are, use the \"--ignore-config\" flag, or the \"--(info/color)-config\" flag and set \nan other file to act as the config file.");
        return "";
    }
}

pub fn get_timezone() -> String {
    let timezone_path = Path::new("/etc/timezone");
    if timezone_path.exists() {
        if let Ok(timezone) = fs::read_to_string(timezone_path) {
            return timezone.trim().to_string();
        }
    }

    let localtime_path = Path::new("/etc/localtime");
    if localtime_path.exists() {
        if let Ok(symlink_target) = fs::read_link(localtime_path) {
            if let Some(target_str) = symlink_target.to_str() {
                if target_str.contains("/zoneinfo/") {
                    if let Some(tz) = target_str.split("/zoneinfo/").last() {
                        return tz.to_string();
                    }
                }
            }
        }
    }

    String::new()
}

pub fn get_cpu_temp() -> String {
    // Try to read temperature on Linux from /sys/class/thermal/thermal_zone*/temp
    let sys_thermal_path = Path::new("/sys/class/thermal/");
    if sys_thermal_path.exists() {
        if let Ok(entries) = fs::read_dir(sys_thermal_path) {
            let temps: Vec<String> = entries
                .par_bridge()
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let path = entry.path();
                    if path.is_dir() && path.to_str()?.contains("thermal_zone") {
                        let temp_path = path.join("temp");
                        if let Ok(temp_str) = fs::read_to_string(temp_path) {
                            if let Ok(temp) = temp_str.trim().parse::<f64>() {
                                return Some(format!("{:.1}°C", temp / 1000.0));
                            }
                        }
                    }
                    None
                })
                .collect();
            if !temps.is_empty() {
                return temps[0].clone();
            }
        }
    }

    let acpi_thermal_path = Path::new("/proc/acpi/thermal_zone/");
    if acpi_thermal_path.exists() {
        if let Ok(entries) = fs::read_dir(acpi_thermal_path) {
            let temps: Vec<String> = entries
                .par_bridge()
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let temp_path = entry.path().join("temperature");
                    if let Ok(file) = File::open(temp_path) {
                        let reader = BufReader::new(file);
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                if let Some(temp_str) = line.split(':').nth(1) {
                                    if let Ok(temp) =
                                        temp_str.trim().split_whitespace().next()?.parse::<f64>()
                                    {
                                        return Some(format!("{:.1}°C", temp));
                                    }
                                }
                            }
                        }
                    }
                    None
                })
                .collect();
            if !temps.is_empty() {
                return temps[0].clone();
            }
        }
    }

    #[cfg(target_os = "netbsd")]
    {
        if let Ok(file) = fs::File::open("/var/run/dmesg.boot") {
            let reader = io::BufReader::new(file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if line.contains("temperature sensor") {
                        if let Some(temp_str) = line.split(':').nth(1) {
                            if let Ok(temp) =
                                temp_str.trim().split_whitespace().next()?.parse::<f64>()
                            {
                                return format!("{:.1}°C", temp);
                            }
                        }
                    }
                }
            }
        }

        if let Ok(output) = std::process::Command::new("sysctl")
            .arg("hw.sensors.cpu0.temp0")
            .output()
        {
            if output.status.success() {
                let output_str = String::from_utf8_lossy(&output.stdout);
                if let Some(temp_str) = output_str.split('=').nth(1) {
                    if let Ok(temp) = temp_str.trim().split_whitespace().next()?.parse::<f64>() {
                        return format!("{:.1}°C", temp);
                    }
                }
            }
        }
    }

    // Return an empty string if temperature could not be determined
    String::new()
}

pub fn get_disk_usage() -> String {
    let output_str = match Command::new("df").arg("-h").output() {
        Ok(output) if output.status.success() => {
            String::from_utf8(output.stdout).unwrap_or_default()
        }
        _ => return String::new(),
    };

    let line = output_str.lines().find(|line| line.starts_with('/'));

    if let Some(line) = line {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            let filesystem = parts[0];
            let used = parts[2];
            let size = parts[1];
            let capacity = parts[4];
            return format!("({}) {} / {} ({})", filesystem, used, size, capacity);
        }
    }

    String::new()
}

pub fn get_gpu_temp() -> String {
    if let Ok(output) = Command::new("nvidia-smi")
        .args(&["--query-gpu=temperature.gpu", "--format=csv,noheader"])
        .output()
    {
        if output.status.success() {
            if let Ok(temp_str) = String::from_utf8(output.stdout) {
                if let Ok(temp) = temp_str.trim().parse::<f64>() {
                    return format!("{:.1}°C", temp);
                }
            }
        }
    }

    if let Ok(entries) = fs::read_dir("/sys/class/drm/") {
        let temps: Vec<_> = entries
            .par_bridge()
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                let temp_path = path.join("device/hwmon/hwmon0/temp1_input");
                fs::read_to_string(temp_path)
                    .ok()
                    .and_then(|temp_str| temp_str.trim().parse::<f64>().ok())
                    .map(|temp| format!("{:.1}°C", temp / 1000.0))
            })
            .collect();
        if let Some(temp) = temps.get(0) {
            return temp.clone();
        }
    }

    #[cfg(target_os = "netbsd")]
    if let Ok(output) = Command::new("sysctl").arg("hw.sensors.gpu0.temp0").output() {
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            if let Some(temp_str) = output_str.split('=').nth(1) {
                if let Ok(temp) = temp_str.trim().split_whitespace().next()?.parse::<f64>() {
                    return format!("{:.1}°C", temp);
                }
            }
        }
    }

    String::new()
}

pub fn get_cpu_info() -> String {
    let cpuinfo = read_to_string("/proc/cpuinfo").expect("Failed to read /proc/cpuinfo");
    let mut cpu = String::new();

    for line in cpuinfo.lines() {
        let parts: Vec<&str> = line.split(": ").map(|s| s.trim()).collect();
        if parts.len() == 2 {
            match parts[0] {
                "model name" | "Hardware" | "Processor" | "^cpu model" | "chip type"
                | "^cpu type" => {
                    cpu = parts[1].to_string();
                    break;
                }
                _ => {}
            }
        }
    }

    format!(
        "{}({})",
        &cpu.split('@').next().unwrap_or_default(),
        get_cpu_temp()
    )
}

fn get_package_managers() -> Vec<&'static str> {
    let possible_managers = vec![
        "xbps-query",
        "dnf",
        "dkpg-query",
        "rpm",
        "apt",
        "pacman",
        "emerge",
        "yum",
        "zypper",
        "apk",
        "pkg_info",
        "pkg",
    ];
    let installed_managers: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    possible_managers.par_iter().for_each(|manager| {
        let version_command = match *manager {
            "pkg_info" => "-V",
            "emerge" => "--help",
            _ => "--version",
        };

        if let Ok(result) = Command::new(manager).arg(version_command).output() {
            if result.status.success() {
                installed_managers.lock().unwrap().push(manager);
            }
        }
    });

    let vec_managers = installed_managers.lock().unwrap().to_vec();
    vec_managers
}

pub fn get_packages() -> String {
    let installed_managers = get_package_managers();
    let packs_numbers: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
    installed_managers.par_iter().for_each(|manager| {
        match *manager {
            "xbps-query" => {
                //xpbs-query -l
                if let Ok(output) = Command::new(*manager)
                    .args(["-l"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "dnf" => {
                // dnf list installed
                if let Ok(output) = Command::new(*manager)
                    .args(["list", "installed"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "dpkg-query" => {
                // dpkg-query -f $(binary:Package) -W
                if let Ok(output) = Command::new(*manager)
                    .args(["-f", "$(binary:Package)", "-W"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "rpm" => {
                // rpm -qa --last
                if let Ok(output) = Command::new(*manager)
                    .args(["-qa", "--last"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "apt" => {
                // apt list --installed
                if let Ok(output) = Command::new("apt-cache")
                    .args(["pkgnames"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "pacman" => {
                // pacman -Q
                if let Ok(output) = Command::new(*manager)
                    .args(["-Q"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "emerge" => {
                // qlist -I
                if get_os_release_pretty_name(None, "ID")
                    .unwrap_or("".to_string())
                    .to_ascii_lowercase()
                    .contains("funtoo")
                {
                    if let Ok(output) = Command::new("find")
                        .args(["/var/db/pkg/"])
                        .args(["-name"])
                        .args(["PF"])
                        .stdout(Stdio::piped())
                        .spawn()
                        .and_then(|child| {
                            Command::new("wc")
                                .args(["-l"])
                                .stdin(child.stdout.unwrap())
                                .output()
                        })
                    {
                        output.status.success().then(|| {
                            if let Some(count) = String::from_utf8(output.stdout)
                                .ok()
                                .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                            {
                                packs_numbers.lock().unwrap().push(count)
                            }
                        });
                    }
                } else if let Ok(output) = Command::new(*manager)
                    .args(["-I"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "yum" => {
                // yum list installed
                if let Ok(output) = Command::new(*manager)
                    .args(["list", "installed"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "zypper" => {
                //zypper se
                if let Ok(output) = Command::new(*manager)
                    .args(["se"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "apk" => {
                // apk list --installed
                if let Ok(output) = Command::new(*manager)
                    .args(["list", "--installed"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "pkg_info" => {
                // ls /usr/pkg/pkgdb/
                if let Ok(output) = Command::new("ls")
                    .args(["/usr/pkg/pkgdb/"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            "pkg" => {
                // pkg info
                if let Ok(output) = Command::new("pkg")
                    .args(["info"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .and_then(|child| {
                        Command::new("wc")
                            .args(["-l"])
                            .stdin(child.stdout.unwrap())
                            .output()
                    })
                {
                    output.status.success().then(|| {
                        if let Some(count) = String::from_utf8(output.stdout)
                            .ok()
                            .and_then(|count_str| count_str.trim().parse::<i16>().ok())
                        {
                            packs_numbers.lock().unwrap().push(count)
                        }
                    });
                }
            }
            _ => {}
        }
    });

    let total_packages: i16 = packs_numbers.lock().unwrap().par_iter().sum();
    total_packages.to_string()
}

pub fn get_res() -> String {
    let output = Command::new("xrandr")
        .arg("--query")
        .output()
        .expect("Failed to execute xrandr command");

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            if let Some(index) = line.find(" connected") {
                let line = &line[index + 1..];
                if let Some(resolution) = line.split_whitespace().find(|s| s.contains('x')) {
                    Some(resolution.split('+').next().unwrap_or("").to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<Vec<String>>()
        .join(", ")
}

pub fn get_gpu_info() -> Result<String, Error> {
    let output = Command::new("lspci").arg("-nnk").output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let reader = BufReader::new(stdout.as_bytes());

    for line in reader.lines() {
        let line = line?;
        if let Some(start_index) = line.find("NVIDIA").or_else(|| line.find("AMD")) {
            let (prefix, prefix_len) = if line.contains("NVIDIA") {
                ("NVIDIA", "NVIDIA".len())
            } else if line.contains("AMD") {
                ("AMD Radeon", "AMD Radeon".len())
            } else {
                let vendor_index = line.find("controller").unwrap_or(0);
                let vendor_str = &line[..vendor_index];
                (vendor_str, 0)
            };
            let start_index = start_index + prefix_len;
            let start_index = line[start_index..].find('[').ok_or(Error::new(
                std::io::ErrorKind::NotFound,
                "GPU name not found",
            ))? + start_index
                + 1;
            let end_index = line[start_index..].find(']').ok_or(Error::new(
                std::io::ErrorKind::NotFound,
                "GPU name not found",
            ))? + start_index;
            let gpu_name = &line[start_index..end_index];
            return Ok(format!(
                "{} {} ({})",
                prefix,
                gpu_name.trim(),
                get_gpu_temp()
            ));
        }
    }

    Err(Error::new(std::io::ErrorKind::NotFound, "GPU not found"))
}

pub fn get_uptime() -> Result<String, Error> {
    let file = File::open("/proc/uptime").expect("Failed to open /proc/uptime");
    let mut reader = BufReader::new(file);
    let mut line = String::new();

    reader
        .read_line(&mut line)
        .expect("Failed to read from /proc/uptime");

    let uptime_secs: f64 = line
        .split_whitespace()
        .next()
        .expect("Failed to parse uptime from /proc/uptime")
        .parse()
        .expect("Failed to parse uptime as f64");

    Ok(format_duration(Duration::from_secs_f64(uptime_secs)))
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let days = seconds / (24 * 3600);
    let hours = (seconds / 3600) % 24;
    let minutes = (seconds / 60) % 60;
    let seconds = seconds % 60;

    let mut uptime_string = String::new();
    if days > 0 {
        uptime_string.push_str(&format!("{} days, ", days));
    }
    if hours > 0 {
        uptime_string.push_str(&format!("{} hours, ", hours));
    }
    if minutes > 0 {
        uptime_string.push_str(&format!("{} minutes, ", minutes));
    }
    uptime_string.push_str(&format!("{} seconds", seconds));
    uptime_string.trim_end_matches(", ").to_string()
}

fn search_file(custom_paths: Vec<&'static str>, search_variable: &str) -> Option<String> {
    for path in custom_paths.iter() {
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                if line.starts_with(search_variable) {
                    if let Some(name) = line.split('=').nth(1) {
                        return Some(name.trim_matches('"').to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn get_os_release_pretty_name(
    overriden_ascii: Option<String>,
    identifier: &str,
) -> Option<String> {
    if overriden_ascii.is_some() {
        return overriden_ascii;
    }

    search_file(vec!["/etc/os-release", "/etc/lsb-release"], identifier)
}

pub fn format_bytes(kbytes: u64) -> String {
    const MIB: u64 = 1048576;
    format!("{:.2} GiB", kbytes as f64 / MIB as f64)
}

pub fn get_wm() -> String {
    if env::var("DISPLAY").is_err() {
        return String::new();
    }

    for env_var in &[
        "XDG_SESSION_DESKTOP",
        "XDG_CURRENT_DESKTOP",
        "DESKTOP_SESSION",
    ] {
        if let Ok(de) = env::var(env_var) {
            return de;
        }
    }

    let path = format!("{}/.xinitrc", env::var("HOME").unwrap_or_default());
    if let Ok(mut file) = File::open(&path) {
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_ok() {
            if let Some(last_line) = buf.lines().last() {
                let last_word = last_line.split(' ').last().unwrap_or("");
                return last_word.to_string();
            }
        }
    }
    String::new()
}

pub fn get_mem() -> String {
    let mem_readout = MemoryReadout::new();
    let total_mem = mem_readout.total().unwrap_or(0);
    let used_mem = mem_readout.used().unwrap_or(0);

    let total_mem_str = format_bytes(total_mem);
    let used_mem_str = format_bytes(used_mem);

    format!("{} / {}", used_mem_str, total_mem_str)
}

pub fn uname_r() -> String {
    let output = Command::new("uname")
        .arg("-r")
        .output()
        .expect("uname failed -r");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn uname_s(overriden_ascii: Option<String>) -> String {
    if overriden_ascii.is_some() {
        return match overriden_ascii {
            Some(str) => str,
            None => String::new(),
        };
    }
    let output = Command::new("uname")
        .arg("-s")
        .output()
        .expect("uname failed -s");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn uname_n() -> String {
    let output = Command::new("uname")
        .arg("-n")
        .output()
        .expect("uname failed -n");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn shell_name() -> String {
    let shell = env::var("SHELL").expect("SHELL not set");
    let parts: Vec<&str> = shell.split('/').collect();
    parts.last().unwrap().to_string()
}

pub fn get_terminal() -> String {
    env::var("TERM").unwrap_or("".to_string())
}
