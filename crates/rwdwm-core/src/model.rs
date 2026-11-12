//! Le modèle du gestionnaire de fenêtres : moniteurs, clients, tags, focus.
//!
//! [`Wm`] est la transposition des structures et fonctions de `dwm.c`. Il est
//! la **seule source de vérité** sur l'emplacement et l'état des clients : le
//! compositeur le modifie par [`Wm::apply`] et les méthodes d'événements, puis
//! le relit pour l'appliquer.
//!
//! Les géométries des clients ([`Client::geom`]) sont **extérieures** (bordure
//! comprise) ; [`Client::inner`] donne celle de la surface.

use std::collections::HashMap;

use crate::action::{Action, Effect};
use crate::bar::{BarView, TagView, BAR_HEIGHT};
use crate::config::{Config, LayoutKind};
use crate::geometry::Rect;
use crate::layout;

/// Identifiant stable d'un client.
pub type ClientId = u32;
/// Identifiant stable d'un moniteur.
pub type MonitorId = u32;
/// Masque de tags : le bit `i` représente le tag d'indice `i`.
pub type TagMask = u32;

/// Fenêtre gérée (`Client` de dwm).
#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    /// Identifiant.
    pub id: ClientId,
    /// Moniteur propriétaire.
    pub mon: MonitorId,
    /// Titre.
    pub title: String,
    /// Identifiant d'application (équivalent de la classe X11).
    pub app_id: String,
    /// Tags portés (jamais nul).
    pub tags: TagMask,
    /// Géométrie extérieure, bordure comprise.
    pub geom: Rect,
    /// Largeur de bordure courante (0 en plein écran).
    pub bw: i32,
    /// Client flottant.
    pub is_floating: bool,
    /// Taille fixe (min = max) : toujours flottant.
    pub is_fixed: bool,
    /// Demande l'attention.
    pub is_urgent: bool,
    /// Plein écran.
    pub is_fullscreen: bool,
    /// Client flottant dont la taille naturelle n'est pas encore connue : sa
    /// géométrie sera fixée par [`Wm::set_natural_size`].
    pub needs_initial_geometry: bool,
    /// Taille intérieure minimale `(w, h)` ; 0 = pas de contrainte.
    pub min_size: (i32, i32),
    /// Taille intérieure maximale `(w, h)` ; 0 = pas de contrainte.
    pub max_size: (i32, i32),
    old_geom: Rect,
    old_bw: i32,
    old_floating: bool,
}

impl Client {
    /// Géométrie de la surface cliente (extérieure moins la bordure).
    pub fn inner(&self) -> Rect {
        self.geom.shrink(self.bw)
    }
}

/// Sortie d'affichage et son état (`Monitor` de dwm).
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// Identifiant.
    pub id: MonitorId,
    /// Nom de la sortie.
    pub name: String,
    /// Géométrie de l'écran (`mx, my, mw, mh`).
    pub screen: Rect,
    /// Écran moins les zones réservées (layer-shell).
    pub usable: Rect,
    /// Zone des fenêtres (`wx, wy, ww, wh`) : `usable` moins la barre.
    pub window_area: Rect,
    /// Barre affichée.
    pub show_bar: bool,
    /// Barre en haut.
    pub top_bar: bool,
    /// Les deux tagsets (courant et précédent).
    pub tagset: [TagMask; 2],
    /// Indice du tagset courant.
    pub seltags: usize,
    /// Les deux dispositions (courante et précédente), indices dans `Config::layouts`.
    pub lt: [usize; 2],
    /// Indice de la disposition courante.
    pub sellt: usize,
    /// Part de la zone maître.
    pub mfact: f32,
    /// Nombre de clients en zone maître.
    pub nmaster: i32,
    /// Facteur d'échelle entier de la sortie.
    pub scale: i32,
    /// Clients dans l'ordre d'attache ; la tête est le maître.
    pub clients: Vec<ClientId>,
    /// Clients dans l'ordre de focus ; la tête est le plus récent.
    pub stack: Vec<ClientId>,
    /// Client sélectionné (toujours visible).
    pub sel: Option<ClientId>,
}

impl Monitor {
    /// Tagset actuellement affiché.
    pub fn current_tagset(&self) -> TagMask {
        self.tagset[self.seltags]
    }

    /// Indice (dans `Config::layouts`) de la disposition courante.
    pub fn current_layout(&self) -> usize {
        self.lt[self.sellt]
    }
}

/// Description d'un client qui apparaît.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewClient {
    /// Identifiant d'application.
    pub app_id: String,
    /// Titre.
    pub title: String,
    /// Client parent (fenêtre transitoire).
    pub parent: Option<ClientId>,
    /// Taille intérieure minimale ; 0 = libre.
    pub min_size: (i32, i32),
    /// Taille intérieure maximale ; 0 = libre.
    pub max_size: (i32, i32),
}

/// Ordre d'empilement des clients visibles d'un moniteur, du fond vers l'avant
/// dans chaque catégorie.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stacking {
    /// Clients tuilés (sous la barre).
    pub tiled: Vec<ClientId>,
    /// Clients flottants (au-dessus de la barre).
    pub floating: Vec<ClientId>,
    /// Clients en plein écran (au-dessus de tout).
    pub fullscreen: Vec<ClientId>,
}

/// L'état complet du gestionnaire de fenêtres.
#[derive(Clone, Debug, PartialEq)]
pub struct Wm {
    config: Config,
    monitors: Vec<Monitor>,
    selmon: usize,
    clients: HashMap<ClientId, Client>,
    status: String,
    next_client: ClientId,
    next_monitor: MonitorId,
}

impl Wm {
    /// Crée un gestionnaire sans moniteur ni client.
    pub fn new(config: Config) -> Self {
        Wm {
            config,
            monitors: Vec::new(),
            selmon: 0,
            clients: HashMap::new(),
            status: concat!("rwdwm-", env!("CARGO_PKG_VERSION")).to_string(),
            next_client: 1,
            next_monitor: 1,
        }
    }

    // ------------------------------------------------------------------
    // Lecture
    // ------------------------------------------------------------------

    /// La configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Tous les moniteurs, dans l'ordre de branchement.
    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    /// Un moniteur par identifiant.
    pub fn monitor(&self, id: MonitorId) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.id == id)
    }

    /// Le moniteur sélectionné (`selmon`).
    pub fn selected_monitor(&self) -> Option<&Monitor> {
        self.monitors.get(self.selmon)
    }

    /// Un client par identifiant.
    pub fn client(&self, id: ClientId) -> Option<&Client> {
        self.clients.get(&id)
    }

    /// Identifiants de tous les clients.
    pub fn client_ids(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients.keys().copied()
    }

    /// Le client sélectionné du moniteur sélectionné.
    pub fn selected(&self) -> Option<ClientId> {
        self.selected_monitor().and_then(|m| m.sel)
    }

    /// Le texte de statut.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Le client est-il affiché (`ISVISIBLE`) ?
    pub fn is_visible(&self, id: ClientId) -> bool {
        self.clients.get(&id).is_some_and(|c| self.monitor(c.mon).is_some_and(|m| c.tags & m.current_tagset() != 0))
    }

    /// Le moniteur contenant le point, à défaut le moniteur sélectionné.
    pub fn monitor_at(&self, x: i32, y: i32) -> Option<MonitorId> {
        self.monitors.iter().find(|m| m.screen.contains(x, y)).or_else(|| self.selected_monitor()).map(|m| m.id)
    }

    /// Rectangle de la barre d'un moniteur, si elle est affichée.
    pub fn bar_rect(&self, mon: MonitorId) -> Option<Rect> {
        let m = self.monitor(mon)?;
        if !m.show_bar {
            return None;
        }
        let y = if m.top_bar { m.usable.y } else { m.usable.bottom() - BAR_HEIGHT };
        Some(Rect::new(m.usable.x, y, m.usable.w, BAR_HEIGHT))
    }

    /// Symbole de disposition affiché dans la barre (`ltsymbol`).
    pub fn layout_symbol(&self, mon: MonitorId) -> String {
        let Some(m) = self.monitor(mon) else { return String::new() };
        let lt = self.config.layouts[m.current_layout()];
        if lt.kind == LayoutKind::Monocle {
            let n = m.clients.iter().filter(|&&c| self.is_visible(c)).count();
            if n > 0 {
                return format!("[{n}]");
            }
        }
        lt.symbol.to_string()
    }

    /// Ordre d'empilement des clients visibles d'un moniteur.
    pub fn stacking(&self, mon: MonitorId) -> Stacking {
        let mut out = Stacking::default();
        let Some(m) = self.monitor(mon) else { return out };
        for &id in m.stack.iter().rev() {
            if !self.is_visible(id) {
                continue;
            }
            let c = &self.clients[&id];
            if c.is_fullscreen {
                out.fullscreen.push(id);
            } else if c.is_floating {
                out.floating.push(id);
            } else {
                out.tiled.push(id);
            }
        }
        out
    }

    /// Contenu de la barre d'un moniteur.
    pub fn bar_view(&self, mon: MonitorId) -> Option<BarView> {
        let m = self.monitor(mon)?;
        let is_selmon = self.selected_monitor().is_some_and(|s| s.id == mon);
        let (mut occupied, mut urgent) = (0, 0);
        for id in &m.clients {
            let c = &self.clients[id];
            occupied |= c.tags;
            if c.is_urgent {
                urgent |= c.tags;
            }
        }
        let sel = m.sel.and_then(|id| self.clients.get(&id));
        let sel_tags = sel.map_or(0, |c| c.tags);
        let tags = self
            .config
            .tags
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let bit = 1 << i;
                TagView {
                    label: label.to_string(),
                    selected: m.current_tagset() & bit != 0,
                    occupied: occupied & bit != 0,
                    urgent: urgent & bit != 0,
                    filled: is_selmon && sel_tags & bit != 0,
                }
            })
            .collect();
        Some(BarView {
            tags,
            layout_symbol: self.layout_symbol(mon),
            title: sel.map(|c| c.title.clone()),
            title_floating: sel.is_some_and(|c| c.is_floating),
            title_fixed: sel.is_some_and(|c| c.is_fixed),
            is_selected_monitor: is_selmon,
            status: is_selmon.then(|| self.status.clone()),
        })
    }

    // ------------------------------------------------------------------
    // Moniteurs
    // ------------------------------------------------------------------

    /// Ajoute un moniteur ; le premier devient le moniteur sélectionné.
    pub fn add_monitor(&mut self, name: &str, screen: Rect) -> MonitorId {
        let id = self.next_monitor;
        self.next_monitor += 1;
        let mut m = Monitor {
            id,
            name: name.to_string(),
            screen,
            usable: screen,
            window_area: screen,
            show_bar: self.config.show_bar,
            top_bar: self.config.top_bar,
            tagset: [1, 1],
            seltags: 0,
            lt: [0, 1 % self.config.layouts.len()],
            sellt: 0,
            mfact: self.config.mfact,
            nmaster: self.config.nmaster,
            scale: 1,
            clients: Vec::new(),
            stack: Vec::new(),
            sel: None,
        };
        for rule in &self.config.monitor_rules {
            if rule.name.is_none_or(|n| name.contains(n)) {
                if let Some(f) = rule.mfact {
                    m.mfact = f.clamp(0.05, 0.95);
                }
                if let Some(n) = rule.nmaster {
                    m.nmaster = n.max(0);
                }
                if let Some(l) = rule.layout.filter(|&l| l < self.config.layouts.len()) {
                    m.lt[0] = l;
                }
                if let Some(s) = rule.scale {
                    m.scale = s.max(1);
                }
            }
        }
        self.monitors.push(m);
        self.update_bar_pos(self.monitors.len() - 1);
        id
    }

    /// Retire un moniteur ; ses clients sont transférés sur le premier moniteur
    /// restant (ils conservent leurs tags).
    ///
    /// Renvoie `false` — sans rien modifier — s'il s'agit du dernier moniteur :
    /// le modèle en garde toujours un pour héberger les clients ; l'appelant
    /// le réutilisera avec [`Wm::rename_monitor`] au prochain branchement.
    pub fn remove_monitor(&mut self, id: MonitorId) -> bool {
        let Some(idx) = self.mon_idx(id) else { return false };
        if self.monitors.len() == 1 {
            return false;
        }
        let removed = self.monitors.remove(idx);
        if self.selmon == idx {
            self.selmon = 0;
        } else if self.selmon > idx {
            self.selmon -= 1;
        }
        let target = self.monitors[0].id;
        // dwm rattache chaque client en tête : l'ordre relatif est conservé en
        // parcourant la liste à l'envers.
        for &cid in removed.clients.iter().rev() {
            if let Some(c) = self.clients.get_mut(&cid) {
                c.mon = target;
            }
            self.monitors[0].clients.insert(0, cid);
        }
        for &cid in removed.stack.iter().rev() {
            self.monitors[0].stack.insert(0, cid);
        }
        for cid in removed.clients {
            self.relocate_floating(cid, removed.window_area);
        }
        self.focus(None);
        self.refocus(0);
        self.arrange_all();
        true
    }

    /// Renomme un moniteur (réutilisation du dernier moniteur après un
    /// débranchement).
    pub fn rename_monitor(&mut self, id: MonitorId, name: &str) {
        if let Some(i) = self.mon_idx(id) {
            self.monitors[i].name = name.to_string();
        }
    }

    /// Met à jour la géométrie d'un moniteur : `screen` est l'écran entier,
    /// `usable` la part non réservée par des surfaces layer-shell.
    pub fn set_monitor_geometry(&mut self, id: MonitorId, screen: Rect, usable: Rect) {
        let Some(i) = self.mon_idx(id) else { return };
        if self.monitors[i].screen == screen && self.monitors[i].usable == usable {
            return;
        }
        let old_area = self.monitors[i].window_area;
        self.monitors[i].screen = screen;
        self.monitors[i].usable = usable;
        self.update_bar_pos(i);
        for cid in self.monitors[i].clients.clone() {
            self.relocate_floating(cid, old_area);
        }
        self.arrange(i);
    }

    // ------------------------------------------------------------------
    // Clients
    // ------------------------------------------------------------------

    /// Prend en charge un nouveau client (`manage` de dwm) : règles, héritage
    /// du parent, attache en tête, focus, placement.
    ///
    /// Renvoie `None` s'il n'existe aucun moniteur.
    pub fn manage(&mut self, new: NewClient) -> Option<ClientId> {
        let selmon = self.monitors.get(self.selmon)?.id;
        let id = self.next_client;
        self.next_client += 1;

        let mut mon = selmon;
        let mut tags = 0;
        let mut is_floating = false;

        let parent = new.parent.and_then(|p| self.clients.get(&p));
        if let Some(p) = parent {
            mon = p.mon;
            tags = p.tags;
            is_floating = true;
        } else {
            for r in &self.config.rules {
                let app_ok = r.app_id.is_none_or(|s| new.app_id.contains(s));
                let title_ok = r.title.is_none_or(|s| new.title.contains(s));
                if app_ok && title_ok {
                    is_floating |= r.is_floating;
                    tags |= r.tags;
                    if let Some(m) = r.monitor.and_then(|i| self.monitors.get(i)) {
                        mon = m.id;
                    }
                }
            }
        }
        let mi = self.mon_idx(mon)?;
        tags &= self.config.tag_mask();
        if tags == 0 {
            tags = self.monitors[mi].current_tagset();
        }

        let is_fixed = is_fixed_size(new.min_size, new.max_size);
        is_floating |= is_fixed;
        let bw = self.config.border_px;
        let area = self.monitors[mi].window_area;
        let (geom, needs_initial_geometry) = if is_fixed {
            (area.centered(new.max_size.0 + 2 * bw, new.max_size.1 + 2 * bw), false)
        } else {
            (area, is_floating)
        };

        self.clients.insert(
            id,
            Client {
                id,
                mon,
                title: new.title,
                app_id: new.app_id,
                tags,
                geom,
                bw,
                is_floating,
                is_fixed,
                is_urgent: false,
                is_fullscreen: false,
                needs_initial_geometry,
                min_size: new.min_size,
                max_size: new.max_size,
                old_geom: geom,
                old_bw: bw,
                old_floating: is_floating,
            },
        );
        self.monitors[mi].clients.insert(0, id);
        self.monitors[mi].stack.insert(0, id);
        self.refocus(mi);
        self.arrange(mi);
        Some(id)
    }

    /// Oublie un client (`unmanage` de dwm) ; le focus revient au client
    /// visible le plus récemment focalisé.
    pub fn unmanage(&mut self, id: ClientId) {
        let Some(c) = self.clients.remove(&id) else { return };
        let Some(mi) = self.mon_idx(c.mon) else { return };
        self.monitors[mi].clients.retain(|&x| x != id);
        self.monitors[mi].stack.retain(|&x| x != id);
        self.refocus(mi);
        self.arrange(mi);
    }

    /// Met à jour le titre d'un client.
    pub fn set_title(&mut self, id: ClientId, title: &str) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.title = title.to_string();
        }
    }

    /// Met à jour l'identifiant d'application d'un client (les règles ne sont
    /// pas réévaluées, comme dans dwm).
    pub fn set_app_id(&mut self, id: ClientId, app_id: &str) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.app_id = app_id.to_string();
        }
    }

    /// Met à jour le texte de statut.
    pub fn set_status(&mut self, status: &str) {
        self.status = status.to_string();
    }

    /// Marque (ou non) un client comme urgent. Le client sélectionné du
    /// moniteur sélectionné ne peut pas devenir urgent.
    pub fn set_urgent(&mut self, id: ClientId, urgent: bool) {
        if urgent && self.selected() == Some(id) {
            return;
        }
        if let Some(c) = self.clients.get_mut(&id) {
            c.is_urgent = urgent;
        }
    }

    /// Met à jour les tailles minimale et maximale d'un client.
    pub fn set_size_limits(&mut self, id: ClientId, min: (i32, i32), max: (i32, i32)) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.min_size = min;
            c.max_size = max;
        }
    }

    /// Communique la taille naturelle (intérieure) d'un client flottant qui
    /// vient d'apparaître : il est centré sur la zone des fenêtres.
    ///
    /// Renvoie `true` si la géométrie a été fixée par cet appel.
    pub fn set_natural_size(&mut self, id: ClientId, w: i32, h: i32) -> bool {
        let Some(c) = self.clients.get(&id) else { return false };
        if !c.needs_initial_geometry || w <= 0 || h <= 0 {
            return false;
        }
        let Some(mi) = self.mon_idx(c.mon) else { return false };
        let area = self.monitors[mi].window_area;
        let c = self.clients.get_mut(&id).expect("client présent");
        c.needs_initial_geometry = false;
        let rect = area.centered(w + 2 * c.bw, h + 2 * c.bw);
        if c.is_fullscreen {
            c.old_geom = rect;
        } else {
            c.geom = rect;
        }
        true
    }

    /// Entre en plein écran ou en sort (`setfullscreen` de dwm).
    pub fn set_fullscreen(&mut self, id: ClientId, on: bool) {
        let Some(c) = self.clients.get_mut(&id) else { return };
        if c.is_fullscreen == on {
            return;
        }
        let mon = c.mon;
        if on {
            c.old_geom = c.geom;
            c.old_bw = c.bw;
            c.old_floating = c.is_floating;
            c.is_fullscreen = true;
            c.is_floating = true;
            c.bw = 0;
        } else {
            c.is_fullscreen = false;
            c.is_floating = c.old_floating;
            c.bw = c.old_bw;
            c.geom = c.old_geom;
        }
        if let Some(mi) = self.mon_idx(mon) {
            self.arrange(mi);
        }
    }

    // ------------------------------------------------------------------
    // Souris
    // ------------------------------------------------------------------

    /// Focalise un client visible (clic, ou focus qui suit la souris) ; son
    /// moniteur devient le moniteur sélectionné.
    pub fn focus_client(&mut self, id: ClientId) {
        if self.is_visible(id) {
            self.focus(Some(id));
        }
    }

    /// Sélectionne un moniteur (la souris y est entrée).
    pub fn select_monitor(&mut self, id: MonitorId) {
        if let Some(i) = self.mon_idx(id) {
            if i != self.selmon {
                self.selmon = i;
                self.focus(None);
            }
        }
    }

    /// Déplacement interactif : place le coin extérieur du client en `(x, y)`
    /// (`movemouse` de dwm : aimantation aux bords de la zone des fenêtres,
    /// détachement d'un client tuilé au-delà du seuil).
    pub fn mouse_move(&mut self, id: ClientId, x: i32, y: i32) {
        let Some(c) = self.clients.get(&id) else { return };
        if c.is_fullscreen {
            return;
        }
        let Some(mi) = self.mon_idx(c.mon) else { return };
        let area = self.monitors[mi].window_area;
        let arranging = self.is_arranging(mi);
        let snap = self.config.snap;
        let (mut nx, mut ny) = (x, y);
        if (area.x - nx).abs() < snap {
            nx = area.x;
        } else if (area.right() - (nx + c.geom.w)).abs() < snap {
            nx = area.right() - c.geom.w;
        }
        if (area.y - ny).abs() < snap {
            ny = area.y;
        } else if (area.bottom() - (ny + c.geom.h)).abs() < snap {
            ny = area.bottom() - c.geom.h;
        }
        let detach = !c.is_floating && arranging && ((nx - c.geom.x).abs() > snap || (ny - c.geom.y).abs() > snap);
        let c = self.clients.get_mut(&id).expect("client présent");
        if detach {
            c.is_floating = true;
        }
        if !arranging || c.is_floating {
            c.geom.x = nx;
            c.geom.y = ny;
        }
        if detach {
            self.arrange(mi);
        }
    }

    /// Redimensionnement interactif : donne au client la taille extérieure
    /// `(w, h)` (`resizemouse` de dwm).
    pub fn mouse_resize(&mut self, id: ClientId, w: i32, h: i32) {
        let Some(c) = self.clients.get(&id) else { return };
        if c.is_fullscreen {
            return;
        }
        let Some(mi) = self.mon_idx(c.mon) else { return };
        let arranging = self.is_arranging(mi);
        let snap = self.config.snap;
        let bw2 = 2 * c.bw;
        let clamp = |v: i32, min: i32, max: i32| {
            let v = v.max(bw2 + 1).max(min + bw2);
            if max > 0 {
                v.min(max + bw2)
            } else {
                v
            }
        };
        let nw = clamp(w, c.min_size.0, c.max_size.0);
        let nh = clamp(h, c.min_size.1, c.max_size.1);
        let detach = !c.is_floating && arranging && ((nw - c.geom.w).abs() > snap || (nh - c.geom.h).abs() > snap);
        let c = self.clients.get_mut(&id).expect("client présent");
        if detach {
            c.is_floating = true;
        }
        if !arranging || c.is_floating {
            c.geom.w = nw;
            c.geom.h = nh;
        }
        if detach {
            self.arrange(mi);
        }
    }

    /// Fin d'un déplacement / redimensionnement : si le client se trouve
    /// majoritairement sur un autre moniteur, il y est transféré.
    pub fn mouse_release(&mut self, id: ClientId) {
        let Some(c) = self.clients.get(&id) else { return };
        let geom = c.geom;
        let from = c.mon;
        let target = self
            .monitors
            .iter()
            .map(|m| (m.screen.intersection_area(&geom), m.id))
            .filter(|&(area, _)| area > 0)
            .max_by_key(|&(area, _)| area)
            .map(|(_, id)| id);
        if let Some(to) = target.filter(|&to| to != from) {
            self.send_to_monitor(id, to, false);
            if let Some(i) = self.mon_idx(to) {
                self.selmon = i;
            }
            self.focus(None);
        }
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    /// Exécute une action et renvoie les effets que le compositeur doit
    /// réaliser (lancer un processus, fermer une fenêtre, quitter…).
    pub fn apply(&mut self, action: &Action) -> Vec<Effect> {
        if self.monitors.is_empty() {
            return match action {
                Action::Spawn(argv) => vec![Effect::Spawn(argv.clone())],
                Action::ChVt(n) => vec![Effect::ChVt(*n)],
                Action::Quit => vec![Effect::Quit],
                _ => Vec::new(),
            };
        }
        let mi = self.selmon;
        let mask = self.config.tag_mask();
        match action {
            Action::Spawn(argv) => return vec![Effect::Spawn(argv.clone())],
            Action::Quit => return vec![Effect::Quit],
            Action::ChVt(n) => return vec![Effect::ChVt(*n)],
            Action::KillClient => return self.selected().map(Effect::Close).into_iter().collect(),
            Action::MoveMouse | Action::ResizeMouse => {
                let Some(id) = self.selected() else { return Vec::new() };
                if self.clients[&id].is_fullscreen {
                    return Vec::new();
                }
                return vec![if *action == Action::MoveMouse {
                    Effect::BeginMove(id)
                } else {
                    Effect::BeginResize(id)
                }];
            }
            Action::ToggleBar => {
                self.monitors[mi].show_bar = !self.monitors[mi].show_bar;
                self.update_bar_pos(mi);
                self.arrange(mi);
            }
            Action::FocusStack(dir) => self.focus_stack(*dir),
            Action::IncNMaster(i) => {
                self.monitors[mi].nmaster = (self.monitors[mi].nmaster + i).max(0);
                self.arrange(mi);
            }
            Action::SetMFact(f) => {
                if !self.is_arranging(mi) {
                    return Vec::new();
                }
                let f = if *f < 1.0 { f + self.monitors[mi].mfact } else { f - 1.0 };
                // Tolérance : 0.55 + 8 × 0.05 doit atteindre 0.95 malgré
                // l'arrondi flottant.
                if !(0.05 - 1e-4..=0.95 + 1e-4).contains(&f) {
                    return Vec::new();
                }
                self.monitors[mi].mfact = f.clamp(0.05, 0.95);
                self.arrange(mi);
            }
            Action::Zoom => self.zoom(),
            Action::View(tags) => {
                let tags = tags & mask;
                if tags == self.monitors[mi].current_tagset() {
                    return Vec::new();
                }
                self.monitors[mi].seltags ^= 1;
                if tags != 0 {
                    let s = self.monitors[mi].seltags;
                    self.monitors[mi].tagset[s] = tags;
                }
                self.focus(None);
                self.arrange(mi);
            }
            Action::ToggleView(tags) => {
                let new = self.monitors[mi].current_tagset() ^ (tags & mask);
                if new != 0 {
                    let s = self.monitors[mi].seltags;
                    self.monitors[mi].tagset[s] = new;
                    self.focus(None);
                    self.arrange(mi);
                }
            }
            Action::Tag(tags) => {
                if let Some(id) = self.selected().filter(|_| tags & mask != 0) {
                    self.clients.get_mut(&id).expect("client présent").tags = tags & mask;
                    self.focus(None);
                    self.arrange(mi);
                }
            }
            Action::ToggleTag(tags) => {
                if let Some(id) = self.selected() {
                    let new = self.clients[&id].tags ^ (tags & mask);
                    if new != 0 {
                        self.clients.get_mut(&id).expect("client présent").tags = new;
                        self.focus(None);
                        self.arrange(mi);
                    }
                }
            }
            Action::SetLayout(layout) => {
                let layout = layout.filter(|&l| l < self.config.layouts.len());
                let m = &mut self.monitors[mi];
                if layout != Some(m.current_layout()) {
                    m.sellt ^= 1;
                }
                if let Some(l) = layout {
                    m.lt[m.sellt] = l;
                }
                self.arrange(mi);
            }
            Action::ToggleFloating => {
                if let Some(id) = self.selected() {
                    let c = self.clients.get_mut(&id).expect("client présent");
                    if !c.is_fullscreen {
                        c.is_floating = !c.is_floating || c.is_fixed;
                        self.arrange(mi);
                    }
                }
            }
            Action::ToggleFullscreen => {
                if let Some(id) = self.selected() {
                    let on = !self.clients[&id].is_fullscreen;
                    self.set_fullscreen(id, on);
                }
            }
            Action::FocusMon(dir) => {
                let target = self.dir_to_mon(*dir);
                if target != self.selmon {
                    self.selmon = target;
                    self.focus(None);
                }
            }
            Action::TagMon(dir) => {
                let target = self.dir_to_mon(*dir);
                if let Some(id) = self.selected().filter(|_| target != self.selmon) {
                    let to = self.monitors[target].id;
                    self.send_to_monitor(id, to, true);
                }
            }
        }
        Vec::new()
    }

    // ------------------------------------------------------------------
    // Vérification
    // ------------------------------------------------------------------

    /// Vérifie les invariants du modèle ; utilisé par les tests après chaque
    /// opération.
    pub fn check_invariants(&self) -> Result<(), String> {
        if !self.monitors.is_empty() && self.selmon >= self.monitors.len() {
            return Err(format!("selmon {} hors limites", self.selmon));
        }
        let mut seen = 0;
        for m in &self.monitors {
            if m.current_tagset() == 0 || m.current_tagset() & !self.config.tag_mask() != 0 {
                return Err(format!("moniteur {} : tagset invalide {:#b}", m.id, m.current_tagset()));
            }
            if !(0.05..=0.95).contains(&m.mfact) || m.nmaster < 0 {
                return Err(format!("moniteur {} : mfact/nmaster invalides", m.id));
            }
            if m.lt.iter().any(|&l| l >= self.config.layouts.len()) {
                return Err(format!("moniteur {} : disposition invalide", m.id));
            }
            let mut a = m.clients.clone();
            let mut b = m.stack.clone();
            a.sort_unstable();
            b.sort_unstable();
            if a != b {
                return Err(format!("moniteur {} : clients et pile de focus divergent", m.id));
            }
            if a.windows(2).any(|w| w[0] == w[1]) {
                return Err(format!("moniteur {} : client en double", m.id));
            }
            for id in &m.clients {
                let c = self.clients.get(id).ok_or(format!("client {id} inconnu"))?;
                if c.mon != m.id {
                    return Err(format!("client {id} : moniteur incohérent"));
                }
                if c.tags == 0 || c.tags & !self.config.tag_mask() != 0 {
                    return Err(format!("client {id} : tags invalides {:#b}", c.tags));
                }
                if c.is_fixed && !c.is_floating {
                    return Err(format!("client {id} : taille fixe mais tuilé"));
                }
                if c.is_fullscreen && (c.bw != 0 || c.geom != m.screen) {
                    return Err(format!("client {id} : plein écran mal placé"));
                }
            }
            match m.sel {
                Some(s) if !m.clients.contains(&s) => return Err(format!("moniteur {} : sel étranger", m.id)),
                Some(s) if !self.is_visible(s) => return Err(format!("moniteur {} : sel invisible", m.id)),
                None if m.clients.iter().any(|&c| self.is_visible(c)) => {
                    return Err(format!("moniteur {} : clients visibles mais pas de sel", m.id))
                }
                _ => {}
            }
            seen += m.clients.len();
        }
        if seen != self.clients.len() {
            return Err(format!("{} clients orphelins", self.clients.len() - seen));
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Interne
    // ------------------------------------------------------------------

    fn mon_idx(&self, id: MonitorId) -> Option<usize> {
        self.monitors.iter().position(|m| m.id == id)
    }

    /// La disposition courante place-t-elle les clients (`lt->arrange != NULL`) ?
    fn is_arranging(&self, mi: usize) -> bool {
        self.config.layouts[self.monitors[mi].current_layout()].kind != LayoutKind::Floating
    }

    /// `dirtomon` de dwm : moniteur suivant / précédent, de façon cyclique.
    fn dir_to_mon(&self, dir: i32) -> usize {
        let n = self.monitors.len();
        if dir > 0 {
            (self.selmon + 1) % n
        } else {
            (self.selmon + n - 1) % n
        }
    }

    /// `updatebarpos` de dwm.
    fn update_bar_pos(&mut self, mi: usize) {
        let m = &mut self.monitors[mi];
        let mut area = m.usable;
        if m.show_bar {
            area.h = (area.h - BAR_HEIGHT).max(0);
            if m.top_bar {
                area.y += BAR_HEIGHT;
            }
        }
        m.window_area = area;
    }

    /// Sélectionne, sur le moniteur `mi`, le premier client visible de la pile
    /// de focus (sans changer de moniteur sélectionné).
    fn refocus(&mut self, mi: usize) {
        let sel = self.monitors[mi].stack.iter().copied().find(|&c| self.is_visible(c));
        self.monitors[mi].sel = sel;
        if let Some(c) = sel.filter(|_| mi == self.selmon) {
            self.clients.get_mut(&c).expect("client présent").is_urgent = false;
        }
    }

    /// `focus` de dwm : focalise `target` s'il est visible, sinon le premier
    /// client visible de la pile de focus du moniteur sélectionné.
    fn focus(&mut self, target: Option<ClientId>) {
        if self.monitors.is_empty() {
            return;
        }
        let target = target
            .filter(|&c| self.is_visible(c))
            .or_else(|| self.monitors[self.selmon].stack.iter().copied().find(|&c| self.is_visible(c)));
        if let Some(id) = target {
            let mon = self.clients[&id].mon;
            if let Some(mi) = self.mon_idx(mon) {
                self.selmon = mi;
                let stack = &mut self.monitors[mi].stack;
                stack.retain(|&x| x != id);
                stack.insert(0, id);
            }
            self.clients.get_mut(&id).expect("client présent").is_urgent = false;
        }
        self.monitors[self.selmon].sel = target;
    }

    fn focus_stack(&mut self, dir: i32) {
        let m = &self.monitors[self.selmon];
        let Some(sel) = m.sel else { return };
        if self.clients[&sel].is_fullscreen && self.config.lock_fullscreen {
            return;
        }
        let visible: Vec<ClientId> = m.clients.iter().copied().filter(|&c| self.is_visible(c)).collect();
        let Some(pos) = visible.iter().position(|&c| c == sel) else { return };
        let n = visible.len();
        let next = if dir > 0 { (pos + 1) % n } else { (pos + n - 1) % n };
        self.focus(Some(visible[next]));
    }

    fn zoom(&mut self) {
        let mi = self.selmon;
        if !self.is_arranging(mi) {
            return;
        }
        let Some(mut c) = self.monitors[mi].sel else { return };
        if self.clients[&c].is_floating {
            return;
        }
        let tiled = self.tiled(mi);
        if tiled.first() == Some(&c) {
            match tiled.get(1) {
                Some(&next) => c = next,
                None => return,
            }
        }
        // `pop` de dwm.
        let clients = &mut self.monitors[mi].clients;
        clients.retain(|&x| x != c);
        clients.insert(0, c);
        self.focus(Some(c));
        self.arrange(mi);
    }

    /// `sendmon` de dwm. Avec `take_tagset`, le client prend le tagset courant
    /// du moniteur d'arrivée.
    fn send_to_monitor(&mut self, id: ClientId, to: MonitorId, take_tagset: bool) {
        let Some(from_i) = self.clients.get(&id).and_then(|c| self.mon_idx(c.mon)) else { return };
        let Some(to_i) = self.mon_idx(to) else { return };
        if from_i == to_i {
            return;
        }
        let old_area = self.monitors[from_i].window_area;
        self.monitors[from_i].clients.retain(|&x| x != id);
        self.monitors[from_i].stack.retain(|&x| x != id);
        let tagset = self.monitors[to_i].current_tagset();
        let c = self.clients.get_mut(&id).expect("client présent");
        c.mon = to;
        if take_tagset {
            c.tags = tagset;
        } else if c.tags & tagset == 0 {
            // Déposé à la souris : il doit rester visible là où on le lâche.
            c.tags = tagset;
        }
        self.monitors[to_i].clients.insert(0, id);
        self.monitors[to_i].stack.insert(0, id);
        if take_tagset {
            self.relocate_floating(id, old_area);
        }
        self.refocus(from_i);
        self.refocus(to_i);
        self.arrange(from_i);
        self.arrange(to_i);
    }

    /// Replace un client flottant dans la zone des fenêtres de son moniteur en
    /// conservant sa position relative à `old_area`.
    fn relocate_floating(&mut self, id: ClientId, old_area: Rect) {
        let Some(c) = self.clients.get(&id) else { return };
        let Some(mi) = self.mon_idx(c.mon) else { return };
        let area = self.monitors[mi].window_area;
        let c = self.clients.get_mut(&id).expect("client présent");
        if !c.is_floating {
            return;
        }
        let g = if c.is_fullscreen { &mut c.old_geom } else { &mut c.geom };
        let x = area.x + (g.x - old_area.x);
        let y = area.y + (g.y - old_area.y);
        g.x = x.min(area.right() - g.w).max(area.x);
        g.y = y.min(area.bottom() - g.h).max(area.y);
    }

    /// Clients tuilés visibles d'un moniteur, dans l'ordre d'attache (`nexttiled`).
    fn tiled(&self, mi: usize) -> Vec<ClientId> {
        self.monitors[mi]
            .clients
            .iter()
            .copied()
            .filter(|&c| self.is_visible(c) && !self.clients[&c].is_floating)
            .collect()
    }

    fn arrange_all(&mut self) {
        for mi in 0..self.monitors.len() {
            self.arrange(mi);
        }
    }

    /// `arrange` de dwm : recalcule la géométrie des clients du moniteur.
    fn arrange(&mut self, mi: usize) {
        let m = &self.monitors[mi];
        let screen = m.screen;
        for id in &m.clients {
            let c = self.clients.get_mut(id).expect("client présent");
            if c.is_fullscreen {
                c.geom = screen;
            }
        }
        let tiled = self.tiled(mi);
        let m = &self.monitors[mi];
        let rects = match self.config.layouts[m.current_layout()].kind {
            LayoutKind::Tile => layout::tile(m.window_area, tiled.len(), m.nmaster, m.mfact),
            LayoutKind::Monocle => layout::monocle(m.window_area, tiled.len()),
            LayoutKind::Floating => return,
        };
        for (id, rect) in tiled.into_iter().zip(rects) {
            self.clients.get_mut(&id).expect("client présent").geom = rect;
        }
    }
}

/// Une taille min = max non nulle désigne un client de taille fixe.
fn is_fixed_size(min: (i32, i32), max: (i32, i32)) -> bool {
    max.0 > 0 && max.1 > 0 && max == min
}

#[cfg(test)]
mod tests;
