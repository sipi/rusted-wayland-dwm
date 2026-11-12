//! Actions liables à une touche ou à un bouton, et effets renvoyés au compositeur.

use crate::model::{ClientId, TagMask};

/// Action déclenchée par un raccourci clavier ou un clic — les « fonctions »
/// du tableau `keys[]` de dwm.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Lancer une commande (`argv`).
    Spawn(Vec<String>),
    /// Afficher / masquer la barre du moniteur sélectionné.
    ToggleBar,
    /// Focaliser le client visible suivant (`+1`) ou précédent (`-1`).
    FocusStack(i32),
    /// Ajouter la valeur à `nmaster` (minimum 0).
    IncNMaster(i32),
    /// Modifier `mfact` : relatif si `< 1.0`, absolu (`valeur - 1.0`) sinon.
    SetMFact(f32),
    /// Promouvoir le client sélectionné en zone maître.
    Zoom,
    /// Afficher le tagset donné ; `0` = tagset précédent.
    View(TagMask),
    /// Ajouter / retirer des tags du tagset affiché.
    ToggleView(TagMask),
    /// Affecter le client sélectionné aux tags donnés.
    Tag(TagMask),
    /// Ajouter / retirer des tags au client sélectionné.
    ToggleTag(TagMask),
    /// Fermer le client sélectionné.
    KillClient,
    /// Choisir la disposition d'indice donné dans `Config::layouts` ;
    /// `None` = revenir à la précédente.
    SetLayout(Option<usize>),
    /// Basculer le client sélectionné entre tuilé et flottant.
    ToggleFloating,
    /// Basculer le plein écran du client sélectionné.
    ToggleFullscreen,
    /// Sélectionner le moniteur suivant (`+1`) ou précédent (`-1`).
    FocusMon(i32),
    /// Envoyer le client sélectionné sur le moniteur suivant / précédent.
    TagMon(i32),
    /// Commencer un déplacement à la souris du client sélectionné.
    MoveMouse,
    /// Commencer un redimensionnement à la souris du client sélectionné.
    ResizeMouse,
    /// Basculer sur le terminal virtuel donné (backend natif).
    ChVt(i32),
    /// Quitter le compositeur.
    Quit,
}

impl Action {
    /// Commande exécutée directement (`execvp`), sans shell.
    pub fn spawn(argv: &[&str]) -> Action {
        Action::Spawn(argv.iter().map(|s| s.to_string()).collect())
    }

    /// Commande exécutée par `/bin/sh -c` — la macro `SHCMD` de dwm.
    pub fn shcmd(cmd: &str) -> Action {
        Action::spawn(&["/bin/sh", "-c", cmd])
    }

    /// Copie de l'action où un masque de tags nul est remplacé par `mask`.
    ///
    /// Sert aux clics sur la barre : dans dwm, un bouton `ClkTagBar` dont
    /// l'argument vaut 0 reçoit le tag cliqué.
    pub fn with_tag_if_unset(&self, mask: TagMask) -> Action {
        match self {
            Action::View(0) => Action::View(mask),
            Action::ToggleView(0) => Action::ToggleView(mask),
            Action::Tag(0) => Action::Tag(mask),
            Action::ToggleTag(0) => Action::ToggleTag(mask),
            other => other.clone(),
        }
    }
}

/// Effet de bord qu'une action demande au compositeur d'exécuter.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Lancer un processus.
    Spawn(Vec<String>),
    /// Demander la fermeture d'un client.
    Close(ClientId),
    /// Arrêter le compositeur.
    Quit,
    /// Changer de terminal virtuel.
    ChVt(i32),
    /// Démarrer un déplacement interactif.
    BeginMove(ClientId),
    /// Démarrer un redimensionnement interactif.
    BeginResize(ClientId),
}
