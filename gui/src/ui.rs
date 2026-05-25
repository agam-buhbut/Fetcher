use std::collections::VecDeque;

use eframe::egui::{self, Color32, RichText, ScrollArea};
use egui_plot::{Line, Plot, PlotPoints};
use taskmgr_core::services::{service_action, ServiceOp, ServiceScope};
use taskmgr_core::startup::{set_enabled, AutostartScope};
use taskmgr_core::{
    human_bytes, kill_process, opt_bytes, KillSignal, ProcessRow, SortColumn, SortOrder,
};

use crate::app::{App, Tab, TICK};

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.maybe_tick();

        // Sidebar (Win-style tab strip)
        egui::SidePanel::left("sidebar")
            .min_width(140.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Task Manager");
                ui.separator();
                for (tab, label) in [
                    (Tab::Processes, "Processes"),
                    (Tab::Performance, "Performance"),
                    (Tab::Startup, "Startup"),
                    (Tab::Services, "Services"),
                ] {
                    if ui
                        .selectable_label(self.tab == tab, format!("  {label}"))
                        .clicked()
                    {
                        self.tab = tab;
                        ctx.request_repaint();
                    }
                }
                ui.add_space(8.0);
                ui.separator();
                if let Some((_, msg)) = &self.status {
                    ui.label(RichText::new(msg).color(Color32::LIGHT_YELLOW));
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Processes => draw_processes(ui, self),
            Tab::Performance => draw_performance(ui, self),
            Tab::Startup => draw_startup(ui, self),
            Tab::Services => draw_services(ui, self),
        });

        // Idle when nothing changes — keeps GUI cheap.
        ctx.request_repaint_after(TICK);
    }
}

fn draw_performance(ui: &mut egui::Ui, app: &App) {
    ui.heading("Performance");
    ui.separator();

    ScrollArea::vertical().show(ui, |ui| {
        if let Some(c) = &app.snapshot.cpu {
            ui.label(format!(
                "CPU  {:.1}%   ({} cores)",
                c.global_usage,
                c.per_core.len()
            ));
        }
        plot_history(ui, "cpu", &app.cpu_history, Some(100.0));

        if let Some(m) = &app.snapshot.memory {
            ui.label(format!(
                "Memory  {} / {}  ({:.1}%)",
                human_bytes(m.used_bytes),
                human_bytes(m.total_bytes),
                m.used_percent()
            ));
        }
        plot_history(ui, "mem", &app.mem_history, Some(100.0));

        ui.label("Disk B/s (R+W)");
        plot_history(ui, "disk", &app.disk_history, None);

        ui.label("Net B/s (RX+TX)");
        plot_history(ui, "net", &app.net_history, None);
    });
}

fn draw_processes(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        ui.heading("Processes");
        ui.add_space(20.0);
        ui.label("Filter:");
        ui.text_edit_singleline(&mut app.filter);
    });
    ui.separator();

    let rows: Vec<ProcessRow> = match &app.snapshot.processes {
        Some(rs) => rs
            .iter()
            .filter(|r| r.matches(&app.filter))
            .cloned()
            .collect(),
        None => Vec::new(),
    };

    let mut new_sort: Option<SortColumn> = None;
    let arrow = |c: SortColumn| -> &'static str {
        if c == app.sort.column {
            match app.sort.order {
                SortOrder::Ascending => " ▲",
                SortOrder::Descending => " ▼",
            }
        } else {
            ""
        }
    };

    ui.horizontal(|ui| {
        for (col, label) in [
            (SortColumn::Pid, "PID"),
            (SortColumn::Name, "Name"),
            (SortColumn::Cpu, "CPU%"),
            (SortColumn::Memory, "Mem"),
            (SortColumn::NetRx, "Net RX"),
            (SortColumn::NetTx, "Net TX"),
        ] {
            if ui.button(format!("{label}{}", arrow(col))).clicked() {
                new_sort = Some(col);
            }
        }
    });
    ui.separator();

    if let Some(c) = new_sort {
        app.sort.cycle(c);
    }

    let mut to_kill: Option<(u32, KillSignal, String)> = None;
    ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("proc_grid")
            .striped(true)
            .num_columns(8)
            .show(ui, |ui| {
                for h in ["PID", "User", "Name", "CPU%", "Mem", "RX/s", "TX/s", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();

                for r in &rows {
                    ui.label(r.pid.to_string());
                    ui.label(&r.user);
                    ui.label(&r.name);
                    ui.label(format!("{:.1}", r.cpu_percent));
                    ui.label(human_bytes(r.memory_bytes));
                    ui.label(opt_bytes(r.net_rx_per_sec));
                    ui.label(opt_bytes(r.net_tx_per_sec));
                    ui.horizontal(|ui| {
                        if ui.small_button("End").clicked() {
                            to_kill = Some((r.pid, KillSignal::Term, r.name.clone()));
                        }
                        if ui.small_button("Force").clicked() {
                            to_kill = Some((r.pid, KillSignal::Kill, r.name.clone()));
                        }
                    });
                    ui.end_row();
                }
            });
    });

    if let Some((pid, sig, name)) = to_kill {
        match kill_process(pid, sig) {
            Ok(()) => app.set_status(format!("sent {sig:?} to {name} ({pid})")),
            Err(e) => app.set_status(format!("kill {pid} failed: {e}")),
        }
    }
}

fn draw_startup(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        ui.heading("Startup");
        if ui.button("Refresh").clicked() {
            app.startup_dirty = true;
        }
    });
    ui.separator();

    let mut to_toggle: Option<(usize, bool)> = None;
    ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("startup_grid")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for h in ["Enabled", "Name", "Scope", "Exec"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, e) in app.autostart.iter().enumerate() {
                    let mut on = e.enabled;
                    if ui.checkbox(&mut on, "").changed() {
                        to_toggle = Some((i, on));
                    }
                    ui.label(&e.name);
                    ui.label(match e.scope {
                        AutostartScope::User => "user",
                        AutostartScope::System => "system",
                    });
                    ui.label(&e.exec);
                    ui.end_row();
                }
            });
    });
    if let Some((i, on)) = to_toggle {
        if let Some(entry) = app.autostart.get(i).cloned() {
            match set_enabled(&entry, on) {
                Ok(updated) => {
                    app.autostart[i] = updated;
                    app.set_status(format!(
                        "{} {}",
                        if on { "enabled" } else { "disabled" },
                        entry.name
                    ));
                }
                Err(e) => app.set_status(format!("toggle failed: {e}")),
            }
        }
    }
}

fn draw_services(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        ui.heading("Services");
        ui.add_space(10.0);
        let user_sel = app.services_scope == ServiceScope::User;
        if ui.selectable_label(user_sel, "User").clicked() {
            app.services_scope = ServiceScope::User;
            app.services_dirty = true;
        }
        if ui.selectable_label(!user_sel, "System").clicked() {
            app.services_scope = ServiceScope::System;
            app.services_dirty = true;
        }
        if ui.button("Refresh").clicked() {
            app.services_dirty = true;
        }
    });
    ui.separator();

    let mut action: Option<(String, ServiceOp)> = None;
    ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("svc_grid")
            .striped(true)
            .num_columns(5)
            .show(ui, |ui| {
                for h in ["Name", "Active", "Sub", "Description", "Actions"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for u in &app.services {
                    ui.label(&u.name);
                    let color = match u.active_state.as_str() {
                        "active" => Color32::LIGHT_GREEN,
                        "failed" => Color32::LIGHT_RED,
                        "inactive" => Color32::GRAY,
                        _ => Color32::YELLOW,
                    };
                    ui.label(RichText::new(&u.active_state).color(color));
                    ui.label(&u.sub_state);
                    ui.label(&u.description);
                    ui.horizontal(|ui| {
                        for (label, op) in [
                            ("Start", ServiceOp::Start),
                            ("Stop", ServiceOp::Stop),
                            ("Restart", ServiceOp::Restart),
                            ("Enable", ServiceOp::Enable),
                            ("Disable", ServiceOp::Disable),
                        ] {
                            if ui.small_button(label).clicked() {
                                action = Some((u.name.clone(), op));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
    });
    if let Some((unit, op)) = action {
        match service_action(&unit, op, app.services_scope) {
            Ok(()) => {
                app.set_status(format!("{op:?} {unit}"));
                app.services_dirty = true;
            }
            Err(e) => app.set_status(format!("{op:?} {unit} failed: {e}")),
        }
    }
}

/// Single plot helper — pass `Some(max)` for fixed axes (percent), `None` to
/// auto-fit (bytes).
fn plot_history(ui: &mut egui::Ui, id: &str, data: &VecDeque<f64>, fixed_max: Option<f64>) {
    let pts: PlotPoints = data
        .iter()
        .enumerate()
        .map(|(i, v)| [i as f64, *v])
        .collect();
    let max = fixed_max.unwrap_or_else(|| data.iter().copied().fold(0.0_f64, f64::max).max(1.0));
    Plot::new(id)
        .height(120.0)
        .include_y(0.0)
        .include_y(max)
        .show_axes([false, true])
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .show(ui, |p| {
            p.line(Line::new(pts).fill(0.0));
        });
}
