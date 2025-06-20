slint::include_modules!();

use std::{path::PathBuf, process::Command, sync::{Arc, Mutex, MutexGuard}};

use slint::{StandardListViewItem, VecModel, ModelRc, SharedString};

mod managed_audio_program;

use managed_audio_program::ManagedAudioProgram;

use crate::managed_audio_program::{read_jack_connections, read_jack_ports, AudioProgramConfig, JackPort};
use std::collections::HashSet;


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
                    filter: "".to_string(),
                    source_name: "system:capture_1".to_string(),
                    target_search_name: "baresip-*".to_string(),
                    target_name: "baresip:input".to_string(),
                }
            ],
        };
        let prog = ManagedAudioProgram {
            config,
            process: None,
            pid_file: PathBuf::new(),
            jack_node_name: "".to_string(),
        };
        prog.save_config();

        audio_programs.lock().unwrap().push(prog);
    }

    let filters = get_filters(audio_programs.lock().unwrap());

    // Alle verfügbaren JACK-Quellen auflisten (z.B. system:capture_1, baresip:input, etc.)
    let jack_sources: Vec<String> = {
        // Verwende die zuvor geparsten Ports (aus `ports`), um die JACK-Quellen zu bestimmen
        ports
            .iter()
            .filter(|port| port.properties.iter().any(|prop| prop == "output"))
            .map(|port| port.name.clone())
            .collect()
    };

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

    ui.set_use_cases(
        ModelRc::new(VecModel::from(
            filters.iter().map(|s| SharedString::from(s.clone())).collect::<Vec<SharedString>>()
        ))
    );

    // Jack-Quellen an Slint übergeben
    let jack_sources_items: Vec<SharedString> = jack_sources
        .into_iter()
        .map(|s| SharedString::from(s))
        .collect();
    ui.set_jack_sources(ModelRc::new(VecModel::from(jack_sources_items)));

    // Jack-Targets an Slint übergeben
    let jack_targets_items: Vec<SharedString> = jack_targets
        .into_iter()
        .map(|s| SharedString::from(s))
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

    // Doppelte Filter-Einträge verhindern
    let unique_filters: Vec<slint::SharedString> = {
        let mut seen = std::collections::HashSet::new();
        filters
            .iter()
            .filter_map(|s| {
                if seen.insert(s) {
                    Some(slint::SharedString::from(s.clone()))
                } else {
                    None
                }
            })
            .collect()
    };
    ui.set_jack_filters(ModelRc::new(
        slint::VecModel::from(unique_filters)
    ));
    ui.set_jack_filter(audio_programs.lock().unwrap().first()
        .and_then(|item| item.config.jack_ports.first())
        .map(|port| port.filter.clone().into())
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
        ui.on_start_use_case(move |use_case| {
            let mut programs = audio_programs.lock().unwrap();
            // Collect indices to connect after mutable borrow ends
            let mut to_connect = Vec::new();
            for (app_index, prog) in programs.iter_mut().enumerate() {
                prog.start();
                for (jack_index, port) in prog.config.jack_ports.iter().enumerate() {
                    if port.filter == use_case.to_string() {
                        to_connect.push((app_index as i32, jack_index as i32));
                    }
                }
            }
            // Drop the mutable borrow before calling connect_jack_ports
            drop(programs);
            let mut programs = audio_programs.lock().unwrap();
            for (app_index, jack_index) in to_connect {
                connect_jack_ports(&mut programs, app_index, jack_index);
            }
            disconnect_unwanted_jack_ports(programs, &use_case);
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();
        ui.on_remove_unwanted_connections(move || {
            if let Ok(programs) = audio_programs.lock() {
                disconnect_unwanted_jack_ports(programs, &"");
            }
            if let Some(ui) = ui_handle.upgrade() {
                let filters = get_filters(audio_programs.lock().unwrap());
                ui.set_use_cases(
                    ModelRc::new(VecModel::from(
                        filters.iter().map(|s| SharedString::from(s.clone())).collect::<Vec<SharedString>>()
                    ))
                );
            }
        });
    }

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
                jack_node_name: "".to_string(),
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
        ui.on_remove_program(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if idx < programs.len() {
                programs[idx].delete_config();
                // Entferne das Programm
                programs.remove(idx);
                // Aktualisiere das Model für Slint
                let items: Vec<StandardListViewItem> = programs
                    .iter()
                    .map(|p| StandardListViewItem::from(SharedString::from(p.config.program_name.clone())))
                    .collect();
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_autio_programs(ModelRc::new(VecModel::from(items)));
                }
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

        // Callback: Programm starten
        ui.on_start_app(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                prog.start();
            }
        });
    }

    {
        // Callback: Jack filter geändert
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();
        ui.on_jack_filter_changed(move || {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let mut programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get_mut(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let filter = ui.get_jack_filter().to_string();
                    // Aktualisiere den Filter für den ausgewählten Port
                    let selected_index = ui.get_Jack_connection_selected();
                    if let Some(port) = prog.config.jack_ports.get_mut(selected_index as usize) {
                        port.filter = filter.clone();

                        let filters = get_filters(programs);
                        let unique_filters: Vec<slint::SharedString> = {
                            let mut seen = std::collections::HashSet::new();
                            filters
                                .iter()
                                .filter_map(|s| {
                                    if seen.insert(s) {
                                        Some(slint::SharedString::from(s.clone()))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        };
                        ui.set_jack_filters(ModelRc::new(
                            slint::VecModel::from(unique_filters)
                        ));
                    }
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

            if let Some(ui) = ui_handle.upgrade() {
                // Filters neu berechnen, nachdem gespeichert wurde
                let filters = get_filters(programs);
                ui.set_use_cases(
                    ModelRc::new(VecModel::from(
                        filters.iter().map(|s| SharedString::from(s.clone())).collect::<Vec<SharedString>>()
                    ))
                );
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
                let source_name = "".to_string();
                let target_name = "".to_string();
                
                prog.config.jack_ports.push(JackPort {
                    filter: "".to_string(),
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
        });
    }

    {
        let audio_programs = audio_programs.clone();
        let ui_handle = ui.as_weak();

        // Callback: Jack-Verbindung ausgewählt
        ui.on_jack_connection_changed(move |_idx | {
            let idx = ui_handle.upgrade().map(|ui| ui.get_program_selected()).unwrap_or(0) as usize;
            let programs = audio_programs.lock().unwrap();
            if let Some(prog) = programs.get(idx) {
                if let Some(ui) = ui_handle.upgrade() {
                    let selected_index = ui.get_Jack_connection_selected() as usize;
                    if selected_index < prog.config.jack_ports.len() {
                        let port = &prog.config.jack_ports[selected_index];
                        ui.set_jack_filter(port.filter.clone().into());
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
        ui.on_jack_connect(move || {
            if let Some(ui) = ui_handle.upgrade() {
                let idx = ui.get_program_selected();
                let mut programs = audio_programs.lock().unwrap();
                let jack_index = ui.get_jack_selected();
                connect_jack_ports(&mut programs, jack_index, idx);
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
            let jack_targets_items: Vec<slint::SharedString> = jack_targets
                .into_iter()
                .map(|s| slint::SharedString::from(s))
                .collect();
            // Im UI setzen
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_jack_targets(slint::ModelRc::new(slint::VecModel::from(jack_targets_items)));
            }
        });
    }

    ui.run().unwrap();
}


fn get_filters(programs: MutexGuard<'_, Vec<ManagedAudioProgram>>) -> Vec<String> {
    let mut filters = {
        let programs = programs;
        let mut all_filters = Vec::new();
        for prog in programs.iter() {
            for port in prog.config.jack_ports.iter() {
                for s in port.filter.split_whitespace() {
                    let s = s.to_string();
                    if !s.is_empty() {
                        all_filters.push(s);
                    }
                }
            }
        }
        let mut seen = HashSet::new();
        all_filters
            .into_iter()
            .filter(|s| seen.insert(s.clone()))
            .collect::<Vec<String>>()
    };
    if filters.is_empty() {
        // Wenn keine Filter vorhanden sind, füge einen Standardfilter hinzu
        filters.push("start".to_string());
    }
    filters
}


pub fn disconnect_unwanted_jack_ports(apps: MutexGuard<'_, Vec<ManagedAudioProgram>>, use_case: &str) -> () {
    let ports = read_jack_ports();
    let connections = read_jack_connections();

    let apps_jack_node_names: Vec<String> = apps.iter().map(|a| a.jack_node_name.clone()).collect();

    let wanted_connections: HashSet<(String, String)> = apps.iter()
        .flat_map(|app| {
            app.config.jack_ports
            .iter()
            .filter(|port| port.filter.split_whitespace().any(|f| f == use_case) || port.filter.is_empty())
            .map(|port| (port.source_name.clone(), get_jack_name(&ports, &apps_jack_node_names, app.jack_node_name.clone(), port)))
        })
        .collect();
    for connection in connections.iter() {
        let source_port = ports.iter().find(|p| p.name == connection.0);
        let target_port = ports.iter().find(|p| p.name == connection.1);
        if let (Some(source), Some(target)) = (source_port, target_port) {
            // Überprüfen, ob die Verbindung unerwünscht ist
            if !wanted_connections.contains(&(connection.0.clone(), connection.1.clone())) {
                // Verbindung trennen
                let output = Command::new("jack_disconnect")
                    .arg(&source.name)
                    .arg(&target.name)
                    .output()
                    .expect("Failed to disconnect JACK ports");
                if output.status.success() {
                    println!("Disconnected {} from {}", source.name, target.name);
                } else {
                    eprintln!("Error disconnecting {} from {}: {}", source.name, target.name, String::from_utf8_lossy(&output.stderr));
                }
            }
        }
    }
}

       
pub fn connect_jack_ports(
    apps: &mut Vec<ManagedAudioProgram>,
    app_index: i32,
    jack_index: i32,
) {
    let ports = read_jack_ports();
    if app_index >= 0 && app_index < apps.len() as i32 {
        let app_index_usize = app_index as usize;
        let app_jack_node_name;
        {
            let app = apps.get(app_index_usize).unwrap();
            app_jack_node_name = app.jack_node_name.clone();
        }
        // Avoid holding a mutable borrow while iterating immutably
        // First, get the port and clone the relevant data
        let port = {
            let app = apps.get(app_index_usize).unwrap();
            app.config.jack_ports.get(jack_index as usize).cloned()
        };
        if let Some(port) = port {
            let source_port = ports.iter().find(|p| p.name == port.source_name);
            let apps_jack_node_names: Vec<String> = apps.iter().map(|a| a.jack_node_name.clone()).collect();
            let search_target = get_jack_name(&ports, &apps_jack_node_names, app_jack_node_name, &port);
            // Now get mutable app only after all immutable borrows are done
            let app = apps.get_mut(app_index_usize).unwrap();
            let target_port = ports.iter().find(|p| p.name == search_target);
            if let (Some(source), Some(target)) = (source_port, target_port) {
                let output = Command::new("jack_connect")
                    .arg(&source.name)
                    .arg(&target.name)
                    .output()
                    .expect("Failed to connect JACK ports");
                if output.status.success() {
                    app.jack_node_name = target_port
                        .as_ref()
                        .map(|p| p.name.split(':').next().unwrap_or(&p.name).to_string())
                        .unwrap_or_default();
                    app.save_jack_target();
                } else {
                    eprintln!("Error connecting {} to {}: {}", source.name, target.name, String::from_utf8_lossy(&output.stderr));
                }
            } else {
                eprintln!("Source or target port not found: {} -> {}", port.source_name, port.target_name);
            }
        }
    };
}


fn get_jack_name(ports: &Vec<managed_audio_program::JackPortInfo>, apps_jack_node_names: &Vec<String>, app_jack_node_name: String, port: &JackPort) -> String {
    let search_target = if !app_jack_node_name.is_empty() {
        app_jack_node_name.clone() + ":" + if port.target_name.contains(':') {
            &port.target_name.split(':').last().unwrap_or(&port.target_name)
        } else {
            &port.target_name
        }
    } else if port.target_name.contains('*') {
        let matching_targets = ports.iter().filter(|p| {
            let pattern = port.target_name.clone();
            let re = regex::Regex::new(&format!("^{}$", pattern)).unwrap();
            re.is_match(&p.name)
        }).collect::<Vec<_>>();
        if !matching_targets.is_empty() {
            let mut first_matching_target = matching_targets.first().unwrap();
            for matching_target in &matching_targets {
                // Use the collected jack_node_names instead of borrowing apps again
                if apps_jack_node_names.iter().any(|name| name == &matching_target.name) {
                    continue;
                }
                first_matching_target = matching_target;
                break; // Nur das erste passende Ziel verbinden
            }
            first_matching_target.name.clone()
        } else {
            port.target_name.clone()
        }
    } else {
        port.target_name.clone()
    };
    search_target
}
