use std::{fs::{self, File}, io::Write, path::PathBuf, process::{Child, Command}};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JackPort {
    pub filter: String,
    pub source_name: String,
    pub target_search_name: String,
    pub target_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioProgramConfig {
    pub program_name: String,
    pub command_name: String,
    pub start_params: Vec<String>,
    pub jack_ports: Vec<JackPort>,
}

pub struct ManagedAudioProgram {
    pub config: AudioProgramConfig,
    pub process: Option<Child>,
    pub pid_file: PathBuf,
    pub jack_node_name: String,
}

impl ManagedAudioProgram {
    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".jackstreamingmanager")
    }
    /// Lädt alle vorhandenen Konfigurationen aus dem Konfigurationsverzeichnis.
    pub fn load_all() -> Vec<Self> {
        let mut programs = Vec::new();
        let config_dir = Self::config_dir();
        if let Ok(entries) = fs::read_dir(&config_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let program_name = path.file_name().unwrap().to_string_lossy().to_string();
                    if let Some(prog) = Self::new(&program_name) {
                        // pid_file beim Laden setzen
                        let mut prog = prog;
                        prog.pid_file = ManagedAudioProgram::pid_path(&program_name);
                        prog.jack_node_name = fs::read_to_string(path.join("jack_target")).unwrap_or_default();
                        programs.push(prog);
                    }
                }
            }
        }
        programs
    }

    /// Lädt die Einstellungen aus der Konfigurationsdatei und gibt eine neue Instanz zurück.
    pub fn new(program_name: &str) -> Option<Self> {
        let config_path = Self::config_path(program_name);
        if let Ok(file) = File::open(&config_path) {
            if let Ok(config) = serde_json::from_reader::<_, AudioProgramConfig>(file) {
                let pid_file = Self::pid_path(program_name);
                return Some(Self {
                    config,
                    process: None,
                    pid_file,
                    jack_node_name: "".to_string(),
                });
            }
        }
        None
    }

    fn config_path(program_name: &str) -> PathBuf {
        Self::config_dir().join(format!("{}/config.json", program_name))
    }

    fn pid_path(program_name: &str) -> PathBuf {
        Self::config_dir().join(format!("{}/pid", program_name))
    }

    pub fn save_config(&self) {
        let dir = Self::config_dir().join(&self.config.program_name);
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.json");
        let file = File::create(config_path).unwrap();
        serde_json::to_writer_pretty(file, &self.config).unwrap();
    }

    pub fn save_pid(&self) {
        if let Some(child) = &self.process {
            let dir = Self::config_dir().join(&self.config.program_name);
            fs::create_dir_all(&dir).unwrap();
            let pid_path = dir.join("pid");
            let mut file = File::create(pid_path).unwrap();
            write!(file, "{}", child.id()).unwrap();
        }
    }

    pub fn save_jack_target(&self) {
        let dir = Self::config_dir().join(&self.config.program_name);
        fs::create_dir_all(&dir).unwrap();
        let target_path = dir.join("jack_target");
        let mut file = File::create(target_path).unwrap();
        write!(file, "{}", self.jack_node_name).unwrap();
    }

    pub fn start(&mut self) {
        // Prüfe, ob das Programm bereits läuft (PID-File vorhanden und Prozess existiert)
        if self.pid_file.exists() {
            if let Ok(pid_str) = fs::read_to_string(&self.pid_file) {
                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    let mut sys = System::new();
                    sys.refresh_processes_specifics(ProcessesToUpdate::All, true,
                        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet).with_exe(UpdateKind::OnlyIfNotSet));
                    if sys.process(sysinfo::Pid::from(pid as usize)).is_some() {
                        println!("Prozess mit PID {} läuft bereits.", pid);
                        return;
                    }
                }
            }
        }
        let mut cmd = Command::new(&self.config.command_name);
        cmd.args(&self.config.start_params);
        let child = cmd.spawn().expect("Failed to start audio program");
        self.process = Some(child);
        self.save_pid();
        // Wenn das gestartete Programm "baresip" ist, sende "D" an stdin
        if self.config.command_name == "baresip" {
            if let Some(child) = &mut self.process {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = write!(stdin, "D");
                    let _ = stdin.flush();
                }
            }
        }
    }

    pub fn delete_config(&self) {
        let dir = Self::config_dir().join(&self.config.program_name);
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("Failed to delete program config directory");
        }
    }
}

#[derive(Debug, Clone)]
pub struct JackPortInfo {
    pub name: String,
    pub properties: Vec<String>,
}

pub fn read_jack_ports() -> Vec<JackPortInfo> {
    let output = Command::new("jack_lsp")
        .arg("-p")
        .output()
        .expect("Failed to execute jack_lsp -p");
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut ports = Vec::new();
    let mut current_port: Option<JackPortInfo> = None;

    for line in stdout.lines() {
        if !line.starts_with('\t') && !line.is_empty() {
            if let Some(port) = current_port.take() {
                ports.push(port);
            }
            current_port = Some(JackPortInfo {
                name: line.trim().to_string(),
                properties: Vec::new(),
            });
        } else if let Some(port) = current_port.as_mut() {
            if let Some(props) = line.trim().strip_prefix("properties:") {
                port.properties = props
                    .split(',')
                    .filter_map(|s| {
                        let s = s.trim();
                        if !s.is_empty() { Some(s.to_string()) } else { None }
                    })
                    .collect();
            }
        }
    }
    if let Some(port) = current_port {
        ports.push(port);
    }
    ports
}

pub fn read_jack_connections() -> Vec<(String, String)> {
    let output = Command::new("jack_lsp")
        .args(["-c", "-p"])
        .output()
        .expect("Failed to execute jack_lsp -c -p");
    let stdout = String::from_utf8_lossy(&output.stdout);

    use std::collections::HashMap;
    let mut port_properties: HashMap<String, Vec<String>> = HashMap::new();
    let mut connections = Vec::new();

    let mut current_port: Option<String> = None;
    let mut last_targets: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') && !line.is_empty() {
            // New port name
            current_port = Some(line.trim().to_string());
        } else if let Some(port) = &current_port {
            let trimmed = line.trim();
            if trimmed.starts_with("properties:") {
                let props = trimmed["properties:".len()..]
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>();
                port_properties.insert(port.clone(), props.clone());
                let last_properties = Some(props);

                let has_output = last_properties
                    .as_ref()
                    .map_or(false, |props| props.iter().any(|p| p == "output"));
                if has_output {
                    for target in last_targets.drain(..) {
                        connections.push((port.clone(), target));
                    }
                }
                last_targets.clear();
            } else if !trimmed.is_empty() {
                last_targets.push(trimmed.to_string());
            }
        }
    }
    connections
}