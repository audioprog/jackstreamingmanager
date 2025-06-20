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
    pub fn load_all() -> (Vec<Self>, Vec<String>) {
        let mut programs = Vec::new();
        let mut errors = Vec::new();
        let config_dir = Self::config_dir();
        if let Ok(entries) = fs::read_dir(&config_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let program_name = path.file_name().unwrap().to_string_lossy().to_string();
                    match Self::new(&program_name) {
                        Some(mut prog) => {
                            prog.pid_file = ManagedAudioProgram::pid_path(&program_name);
                            prog.jack_node_name = fs::read_to_string(path.join("jack_target")).unwrap_or_default();
                            if let Err(e) = prog.remove_dead_pids() {
                                errors.extend(e);
                            }
                            programs.push(prog);
                        }
                        None => {
                            errors.push(format!("Fehler beim Laden der Konfiguration für '{}'", program_name));
                        }
                    }
                }
            }
        } else {
            errors.push(format!("Fehler beim Lesen des Konfigurationsverzeichnisses: {:?}", config_dir));
        }
        (programs, errors)
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

    pub fn save_config(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let dir = Self::config_dir().join(&self.config.program_name);
        if let Err(e) = fs::create_dir_all(&dir) {
            errors.push(format!("Fehler beim Erstellen des Verzeichnisses: {}", e));
        }
        let config_path = dir.join("config.json");
        match File::create(&config_path) {
            Ok(file) => {
                if let Err(e) = serde_json::to_writer_pretty(file, &self.config) {
                    errors.push(format!("Fehler beim Schreiben der Konfiguration: {}", e));
                }
            }
            Err(e) => errors.push(format!("Fehler beim Erstellen der Konfigurationsdatei: {}", e)),
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn save_pid(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if let Some(child) = &self.process {
            let dir = Self::config_dir().join(&self.config.program_name);
            if let Err(e) = fs::create_dir_all(&dir) {
                errors.push(format!("Fehler beim Erstellen des Verzeichnisses: {}", e));
            }
            let pid_path = dir.join("pid");
            match File::create(&pid_path) {
                Ok(mut file) => {
                    if let Err(e) = write!(file, "{}", child.id()) {
                        errors.push(format!("Fehler beim Schreiben der PID: {}", e));
                    }
                }
                Err(e) => errors.push(format!("Fehler beim Erstellen der PID-Datei: {}", e)),
            }
        } else {
            errors.push("Kein laufender Prozess vorhanden.".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn save_jack_target(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let dir = Self::config_dir().join(&self.config.program_name);
        if let Err(e) = fs::create_dir_all(&dir) {
            errors.push(format!("Fehler beim Erstellen des Verzeichnisses: {}", e));
        }
        let target_path = dir.join("jack_target");
        match File::create(&target_path) {
            Ok(mut file) => {
                if let Err(e) = write!(file, "{}", self.jack_node_name) {
                    errors.push(format!("Fehler beim Schreiben der jack_node_name: {}", e));
                }
            }
            Err(e) => errors.push(format!("Fehler beim Erstellen der Datei: {}", e)),
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn remove_dead_pids(&mut self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.pid_file.exists() {
            match fs::read_to_string(&self.pid_file) {
                Ok(pid_str) => {
                    match pid_str.trim().parse::<u32>() {
                        Ok(pid) => {
                            let mut sys = System::new();
                            sys.refresh_processes_specifics(
                                ProcessesToUpdate::All,
                                true,
                                ProcessRefreshKind::nothing()
                                    .with_cmd(UpdateKind::OnlyIfNotSet)
                                    .with_exe(UpdateKind::OnlyIfNotSet),
                            );
                            if sys.process(sysinfo::Pid::from(pid as usize)).is_none() {
                                if let Err(e) = fs::remove_file(&self.pid_file) {
                                    errors.push(format!("Fehler beim Entfernen der PID-Datei: {}", e));
                                }
                            }
                        }
                        Err(e) => errors.push(format!("Fehler beim Parsen der PID: {}", e)),
                    }
                }
                Err(e) => errors.push(format!("Fehler beim Lesen der PID-Datei: {}", e)),
            }
        } else {
            errors.push("PID-Datei existiert nicht.".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn start(&mut self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Prüfe, ob das Programm bereits läuft (PID-File vorhanden und Prozess existiert)
        if self.pid_file.exists() {
            match fs::read_to_string(&self.pid_file) {
                Ok(pid_str) => {
                    match pid_str.trim().parse::<u32>() {
                        Ok(pid) => {
                            let mut sys = System::new();
                            sys.refresh_processes_specifics(
                                ProcessesToUpdate::All,
                                true,
                                ProcessRefreshKind::nothing()
                                    .with_cmd(UpdateKind::OnlyIfNotSet)
                                    .with_exe(UpdateKind::OnlyIfNotSet),
                            );
                            let test = sys.process(sysinfo::Pid::from(pid as usize));
                            if test.is_some() && test.unwrap().name().to_string_lossy().contains(&self.config.command_name) {
                                errors.push(format!("Prozess mit PID {} läuft bereits.", pid));
                                return Err(errors);
                            }
                        }
                        Err(e) => errors.push(format!("Fehler beim Parsen der PID: {}", e)),
                    }
                }
                Err(e) => errors.push(format!("Fehler beim Lesen der PID-Datei: {}", e)),
            }
        }

        let mut cmd = Command::new(&self.config.command_name);
        cmd.args(&self.config.start_params);
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                errors.push(format!("Fehler beim Starten des Audio-Programms: {}", e));
                return Err(errors);
            }
        };
        self.process = Some(child);
        if let Err(pid_errors) = self.save_pid() {
            errors.extend(pid_errors);
        }

        // Pause für 300 ms nach dem Starten des Prozesses
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Wenn das gestartete Programm "baresip" ist, sende "D" an stdin
        if self.config.command_name == "baresip" {
            if let Some(child) = &mut self.process {
                if let Some(stdin) = child.stdin.as_mut() {
                    if let Err(e) = write!(stdin, "D") {
                        errors.push(format!("Fehler beim Schreiben an baresip stdin: {}", e));
                    }
                    if let Err(e) = stdin.flush() {
                        errors.push(format!("Fehler beim Flushen von baresip stdin: {}", e));
                    }
                } else {
                    errors.push("baresip stdin ist nicht verfügbar.".to_string());
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
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