slint::include_modules!();

use std::process::{Command, Stdio, Child, ChildStdin};
use std::io::{BufReader, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::fs::{self, File};
use std::path::{Path, PathBuf};



// Add serde derive macros
use serde::{Serialize, Deserialize};
use slint::{StandardListViewItem, VecModel, ModelRc, SharedString};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};


#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct JackPort {
    source_name: String,
    target_search_name: String,
    target_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AudioProgramConfig {
    program_name: String,
    command_name: String,
    start_params: Vec<String>,
    jack_ports: Vec<JackPort>,
}

struct ManagedAudioProgram {
    config: AudioProgramConfig,
    process: Option<Child>,
    pid_file: PathBuf,
}

impl ManagedAudioProgram {
    fn config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".jackstreamingmanager")
    }
    /// Lädt alle vorhandenen Konfigurationen aus dem Konfigurationsverzeichnis.
    fn load_all() -> Vec<Self> {
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
                        programs.push(prog);
                    }
                }
            }
        }
        programs
    }

    /// Lädt die Einstellungen aus der Konfigurationsdatei und gibt eine neue Instanz zurück.
    fn new(program_name: &str) -> Option<Self> {
        let config_path = Self::config_path(program_name);
        if let Ok(file) = File::open(&config_path) {
            if let Ok(config) = serde_json::from_reader::<_, AudioProgramConfig>(file) {
                let pid_file = Self::pid_path(program_name);
                return Some(Self {
                    config,
                    process: None,
                    pid_file,
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

    fn save_config(&self) {
        let dir = Self::config_dir().join(&self.config.program_name);
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.json");
        let file = File::create(config_path).unwrap();
        serde_json::to_writer_pretty(file, &self.config).unwrap();
    }

    fn save_pid(&self) {
        if let Some(child) = &self.process {
            let dir = Self::config_dir().join(&self.config.program_name);
            fs::create_dir_all(&dir).unwrap();
            let pid_path = dir.join("pid");
            let mut file = File::create(pid_path).unwrap();
            write!(file, "{}", child.id()).unwrap();
        }
    }

    fn start(&mut self) {
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
}

#[derive(Debug, Clone)]
struct JackPortInfo {
    name: String,
    properties: Vec<String>,
}

fn read_jack_ports() -> Vec<JackPortInfo> {
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

fn main() {
    // Ports beim Start einlesen
    let ports = read_jack_ports();

    // Programme verwalten
    // Hier alle vorhandenen Konfigurationen laden
    let audio_programs: Arc<Mutex<Vec<ManagedAudioProgram>>> = Arc::new(Mutex::new(
        ManagedAudioProgram::load_all()
    ));

    // Beispiel: Programm hinzufügen und starten
    // Nur Beispiel hinzufügen, wenn keine Programme geladen wurden
    if audio_programs.lock().unwrap().is_empty() {
        let config = AudioProgramConfig {
            program_name: "baresip stream".to_string(),
            command_name: "baresip".to_string(),
            start_params: vec![],
            jack_ports: vec![
                JackPort {
                    source_name: "system:capture_1".to_string(),
                    target_search_name: "baresip-*".to_string(),
                    target_name: "baresip:input".to_string(),
                }
            ],
        };
        let mut prog = ManagedAudioProgram {
            config,
            process: None,
            pid_file: PathBuf::new(),
        };
        prog.save_config();

        audio_programs.lock().unwrap().push(prog);
    }
    // Alle verfügbaren JACK-Quellen auflisten (z.B. system:capture_1, baresip:input, etc.)
    let jack_sources: Vec<String> = {
        // Verwende die zuvor geparsten Ports (aus `ports`), um die JACK-Quellen zu bestimmen
        ports
            .iter()
            .filter(|port| port.properties.iter().any(|prop| prop == "output"))
            .map(|port| port.name.clone())
            .collect()
    };

    println!("Verfügbare JACK-Quellen:");
    for src in &jack_sources {
        println!("  {}", src);
    }

    let jack_targets: Vec<String> = {
        // Verwende die zuvor geparsten Ports (aus `ports`), um die JACK-Quellen zu bestimmen
        ports
            .iter()
            .filter(|port| port.properties.iter().any(|prop| prop == "input"))
            .map(|port| port.name.clone())
            .collect()
    };

    let jack_connections: Vec<String> = audio_programs.lock().unwrap().first()
        .map(|prog| {
            prog.config.jack_ports.iter()
                .map(|port| format!("{} -> {}", port.source_name, port.target_name))
                .collect()
        })
        .unwrap_or_default();

    let ui = MainWindow::new().unwrap();

    // Jack-Quellen an Slint übergeben
    let jack_sources_items: Vec<StandardListViewItem> = jack_sources
        .into_iter()
        .map(|s| StandardListViewItem::from(SharedString::from(s)))
        .collect();
    ui.set_jack_sources(ModelRc::new(VecModel::from(jack_sources_items)));

    // Jack-Targets an Slint übergeben
    let jack_targets_items: Vec<StandardListViewItem> = jack_targets
        .into_iter()
        .map(|s| StandardListViewItem::from(SharedString::from(s)))
        .collect();
    ui.set_jack_targets(ModelRc::new(VecModel::from(jack_targets_items)));

    let jack_connections_items: Vec<StandardListViewItem> = jack_connections
        .into_iter()
        .map(|s| StandardListViewItem::from(SharedString::from(s)))
        .collect();
    ui.set_jack_connections(ModelRc::new(VecModel::from(jack_connections_items)));

    // Programme als Model für Slint
    let program_items: Vec<StandardListViewItem> = audio_programs
        .lock().unwrap()
        .iter()
        .map(|p| StandardListViewItem::from(SharedString::from(p.config.program_name.clone())))
        .collect();

    ui.set_autio_programs(ModelRc::new(VecModel::from(program_items)));

    ui.set_edit_program_name(audio_programs.lock().unwrap().first()
        .map(|item| item.config.program_name.clone().into())
        .unwrap_or_default()
    );
    ui.set_edit_command_name(audio_programs.lock().unwrap().first()
        .map(|item| item.config.command_name.clone().into())
        .unwrap_or_default()
    );
    ui.set_edit_start_params(audio_programs.lock().unwrap().first()
        .map(|item| item.config.start_params.join(" ").into())
        .unwrap_or_default()
    );

    ui.set_jack_source(audio_programs.lock().unwrap().first()
        .and_then(|item| item.config.jack_ports.first())
        .map(|port| port.source_name.clone().into())
        .unwrap_or_default()
    );
    ui.set_jack_target(audio_programs.lock().unwrap().first()
        .and_then(|item| item.config.jack_ports.first())
        .map(|port| port.target_name.clone().into())
        .unwrap_or_default()
    );
    ui.set_jack_search(audio_programs.lock().unwrap().first()
        .and_then(|item| item.config.jack_ports.first())
        .map(|port| port.target_search_name.clone().into())
        .unwrap_or_default()
    );

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();
        ui.on_add_program(move || {
            let mut programs = audio_programs.lock().unwrap();
            let new_name = format!("Neues Programm {}", programs.len() + 1);
            let config = AudioProgramConfig {
                program_name: new_name.clone(),
                command_name: "".to_string(),
                start_params: vec![],
                jack_ports: vec![],
            };
            let prog = ManagedAudioProgram {
                config,
                process: None,
                pid_file: PathBuf::new(),
            };
            programs.push(prog);

            // Model für Slint aktualisieren
            let items: Vec<StandardListViewItem> = programs
                .iter()
                .map(|p| StandardListViewItem::from(SharedString::from(p.config.program_name.clone())))
                .collect();
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_autio_programs(ModelRc::new(VecModel::from(items)));
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();
        ui.on_program_selectiion_changed(move |idx| {
            let programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get(idx as usize) {
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_edit_program_name(prog.config.program_name.clone().into());
                    ui.set_edit_command_name(prog.config.command_name.clone().into());
                    ui.set_edit_start_params(prog.config.start_params.join(" ").into());
                }
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Programmname geändert
        ui.on_program_name_changed(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    prog.config.program_name = ui.get_edit_program_name().to_string();
                }
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Kommando geändert
        ui.on_command_name_changed(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    prog.config.command_name = ui.get_edit_command_name().to_string();
                }
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Startparameter geändert
        ui.on_start_params_changed(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    prog.config.start_params = ui.get_edit_start_params().split_whitespace().map(|s| s.to_string()).collect();
                }
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Speichern-Button
        ui.on_save_settings(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                prog.save_config();
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Jack-Verbindung hinzufügen
        ui.on_jack_connection_add(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let source_name = "".to_string();
                    let target_name = "".to_string();
                    
                    prog.config.jack_ports.push(JackPort {
                        source_name,
                        target_search_name: target_name.clone(),
                        target_name,
                    });
                    prog.save_config();
                    // Aktualisiere die Verbindungen im UI
                    let connections: Vec<StandardListViewItem> = prog.config.jack_ports.iter()
                        .map(|port| StandardListViewItem::from(SharedString::from(format!("{} -> {}", port.source_name, port.target_name))))
                        .collect();
                    if let Some(ui) = ui_handle.upgrade() {
                        ui.set_jack_connections(ModelRc::new(VecModel::from(connections)));
                    }
                }
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Jack-Verbindung ausgewählt
        ui.on_jack_connection_changed(move |idx | {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let selected_index = ui.get_Jack_connection_selected() as usize;
                    if selected_index < prog.config.jack_ports.len() {
                        let port = &prog.config.jack_ports[selected_index];
                        ui.set_jack_source(port.source_name.clone().into());
                        ui.set_jack_target(port.target_name.clone().into());
                        ui.set_jack_search(port.target_search_name.clone().into());
                    }
                }
            }
        });
    }

    {
        // Callback: Jack-Quelle geändert
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();
        ui.on_jack_source_changed(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(-1) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let connection_idx = ui.get_Jack_connection_selected() as usize;
                    if let Some(port) = prog.config.jack_ports.get_mut(connection_idx) {
                        let source_name = ui.get_jack_source().to_string();
                        port.source_name = source_name;
                        prog.save_config();

                        // Aktualisiere die Verbindungen im UI
                        let connections: Vec<StandardListViewItem> = prog.config.jack_ports.iter()
                            .map(|port| StandardListViewItem::from(SharedString::from(format!("{} -> {}", port.source_name, port.target_name))))
                            .collect();
                        ui.set_jack_connections(ModelRc::new(VecModel::from(connections)));
                    }
                }
            }
        });
    }

    {
        // Callback: Jack-Ziel geändert
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();
        ui.on_jack_target_changed(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let connection_idx = ui.get_Jack_connection_selected() as usize;
                    if let Some(port) = prog.config.jack_ports.get_mut(connection_idx) {
                        let target_name = ui.get_jack_target().to_string();
                        port.target_name = target_name;
                        prog.save_config();

                        // Aktualisiere die Verbindungen im UI
                        let connections: Vec<StandardListViewItem> = prog.config.jack_ports.iter()
                            .map(|port| StandardListViewItem::from(SharedString::from(format!("{} -> {}", port.source_name, port.target_name))))
                            .collect();
                        ui.set_jack_connections(ModelRc::new(VecModel::from(connections)));
                    }
                }
            }
        });
    }

    {
        // Callback: Jack-Suche geändert
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();
        ui.on_jack_search_changed(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let connection_idx = ui.get_Jack_connection_selected() as usize;
                    if let Some(port) = prog.config.jack_ports.get_mut(connection_idx) {
                        let target_search_name = ui.get_jack_search().to_string();
                        port.target_search_name = target_search_name;
                        prog.save_config();

                        // Aktualisiere die Verbindungen im UI
                        let connections: Vec<StandardListViewItem> = prog.config.jack_ports.iter()
                            .map(|port| StandardListViewItem::from(SharedString::from(format!("{} -> {}", port.source_name, port.target_name))))
                            .collect();
                        ui.set_jack_connections(ModelRc::new(VecModel::from(connections)));
                    }
                }
            }
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Jack-Verbindung entfernen
        ui.on_jack_connection_remove(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let selected_index = ui.get_Jack_connection_selected() as usize;
                    if selected_index < prog.config.jack_ports.len() {
                        prog.config.jack_ports.remove(selected_index);
                        prog.save_config();
                        // Aktualisiere die Verbindungen im UI
                        let connections: Vec<StandardListViewItem> = prog.config.jack_ports.iter()
                            .map(|port| StandardListViewItem::from(SharedString::from(format!("{} -> {}", port.source_name, port.target_name))))
                            .collect();
                        let connections_len = connections.len();
                        ui.set_jack_connections(ModelRc::new(VecModel::from(connections)));

                        // Setze die Eingabefelder zurück
                        ui.set_Jack_connection_selected(connections_len as i32 - 1);
                    }
                }
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        ui.on_jack_target_reinit(move || {
            // JACK-Targets neu abfragen
            let ports = read_jack_ports();
            let jack_targets: Vec<String> = {
                // Verwende die zuvor geparsten Ports (aus `ports`), um die JACK-Quellen zu bestimmen
                ports
                    .iter()
                    .filter(|port| port.properties.iter().any(|prop| prop == "input"))
                    .map(|port| port.name.clone())
                    .collect()
            };
            // In StandardListViewItem umwandeln
            let jack_targets_items: Vec<slint::StandardListViewItem> = jack_targets
                .into_iter()
                .map(|s| slint::StandardListViewItem::from(slint::SharedString::from(s)))
                .collect();
            // Im UI setzen
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_jack_targets(slint::ModelRc::new(slint::VecModel::from(jack_targets_items)));
            }
        });
    }

    ui.run();
}