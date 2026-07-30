//! Relais d'apparence joueur (B3, modèle PRESET — décision 2026-07-25). Le serveur autoritaire tient
//! l'apparence choisie de chaque joueur (un PRESET = `base_record` + `appearance`, deux hashes) et
//! décide **QUI doit recevoir un `AppearanceSync`** : à l'entrée en zone d'intérêt (AoI) —
//! auto-cicatrisant, l'arrivant tardif reçoit l'apparence de ceux déjà en portée — et à chaque
//! changement d'apparence.
//!
//! Module PUR : aucune I/O, aucune dépendance FlatBuffers. Il ne fait que la LOGIQUE de « qui a
//! besoin de quoi » ; l'encodage `AppearanceSync` et l'envoi sont câblés par `server_loop`. Même
//! esprit que `escalade_police.rs` (pur, testable en isolation). L'apparence est un canal RARE (hors
//! Snapshot 20 Hz, cf. protocole `AppearanceSync`) : ce module ne renvoie un spec que quand c'est
//! réellement nouveau pour l'observateur, jamais en boucle.
//!
//! Le descripteur d'index de customisation (visage exact) est DIFFÉRÉ (chantier natif, cf. design
//! apparence §6.2) — ici on ne transporte que le preset, qui suffit au playtest et se RENd (une
//! entité NPC preset s'affiche, contrairement à un corps joueur — prouvé en jeu 2026-07-25).

use crate::transport::ClientId;
use std::collections::{HashMap, HashSet};

/// Apparence PRESET d'un joueur : `base_record` (hash TweakDBID de l'entité de base à spawner) +
/// `appearance` (hash CName de la variante appliquée par `ScheduleAppearanceChange` côté client).
/// Deux `u64` suffisent — pas de blob opaque, pas de descripteur d'index (différé, design §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppearanceSpec {
    pub base_record: u64,
    pub appearance: u64,
}

/// Décide qui reçoit quel `AppearanceSync`. Ne tient AUCUN état réseau — juste l'apparence connue de
/// chaque joueur et ce que chaque observateur a déjà reçu, pour ne pas ré-émettre inutilement.
#[derive(Debug, Default)]
pub struct AppearanceRelay {
    /// Apparence courante de chaque joueur (absent = pas encore d'apparence choisie).
    specs: HashMap<ClientId, AppearanceSpec>,
    /// Pour chaque observateur, l'ensemble des sujets dont il détient déjà l'apparence À JOUR.
    delivered: HashMap<ClientId, HashSet<ClientId>>,
}

impl AppearanceRelay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pose ou met à jour l'apparence d'un joueur. Si elle CHANGE réellement, on l'oublie chez tous
    /// les observateurs (ils la re-recevront à leur prochain `pending_for` s'ils sont en portée) —
    /// c'est le rejeu « à chaque changement ». Poser la même valeur est un no-op (pas de re-spam).
    pub fn set(&mut self, subject: ClientId, spec: AppearanceSpec) {
        if self.specs.get(&subject) == Some(&spec) {
            return;
        }
        self.specs.insert(subject, spec);
        for seen in self.delivered.values_mut() {
            seen.remove(&subject);
        }
    }

    /// Retire un joueur (déconnexion). Son apparence disparaît et on nettoie les traces de livraison.
    pub fn remove(&mut self, subject: ClientId) {
        self.specs.remove(&subject);
        self.delivered.remove(&subject);
        for seen in self.delivered.values_mut() {
            seen.remove(&subject);
        }
    }

    /// Lecture seule de l'apparence connue d'un joueur (pour un push ciblé hors AoI si besoin).
    pub fn get(&self, subject: ClientId) -> Option<AppearanceSpec> {
        self.specs.get(&subject).copied()
    }

    /// Pour un observateur donné et la liste des sujets actuellement dans son AoI, renvoie les
    /// `(sujet, apparence)` qu'il n'a PAS encore reçus (nouvel arrivant OU apparence changée), et les
    /// marque comme livrés. Un sujet sorti de l'AoI est oublié (re-entrée => renvoi, auto-cicatrisant).
    /// L'observateur ne reçoit jamais sa propre apparence, ni celle d'un joueur sans apparence posée.
    pub fn pending_for(
        &mut self,
        observer: ClientId,
        in_aoi: &[ClientId],
    ) -> Vec<(ClientId, AppearanceSpec)> {
        let seen = self.delivered.entry(observer).or_default();
        let mut out = Vec::new();
        let mut now_visible: HashSet<ClientId> = HashSet::new();
        for &subject in in_aoi {
            if subject == observer {
                continue;
            }
            let Some(&spec) = self.specs.get(&subject) else {
                continue;
            };
            now_visible.insert(subject);
            if !seen.contains(&subject) {
                out.push((subject, spec));
            }
        }
        // Ne garder comme « livrés » que les sujets encore en portée ET avec une apparence : ceux
        // sortis de l'AoI seront ré-émis à la ré-entrée (auto-cicatrisant).
        *seen = now_visible;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ClientId = 1;
    const B: ClientId = 2;
    const C: ClientId = 3;

    fn spec(n: u64) -> AppearanceSpec {
        AppearanceSpec {
            base_record: n,
            appearance: n + 1000,
        }
    }

    #[test]
    fn a_subject_in_aoi_is_pending_once_then_not_again() {
        let mut r = AppearanceRelay::new();
        r.set(B, spec(1));
        // A voit B pour la première fois -> B est en attente.
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(1))]);
        // Rien de neuf au tour suivant.
        assert_eq!(r.pending_for(A, &[B]), vec![]);
    }

    #[test]
    fn a_late_joiner_receives_the_appearance_of_those_already_in_range() {
        let mut r = AppearanceRelay::new();
        r.set(B, spec(2));
        r.set(C, spec(3));
        // A arrive : il doit recevoir B ET C, déjà présents.
        let mut got = r.pending_for(A, &[B, C]);
        got.sort_by_key(|(id, _)| *id);
        assert_eq!(got, vec![(B, spec(2)), (C, spec(3))]);
    }

    #[test]
    fn a_changed_appearance_is_re_sent_to_observers_who_had_it() {
        let mut r = AppearanceRelay::new();
        r.set(B, spec(1));
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(1))]);
        // B change d'apparence -> A doit la recevoir de nouveau.
        r.set(B, spec(9));
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(9))]);
    }

    #[test]
    fn re_setting_the_same_appearance_does_not_re_send() {
        let mut r = AppearanceRelay::new();
        r.set(B, spec(1));
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(1))]);
        r.set(B, spec(1)); // même valeur -> no-op
        assert_eq!(r.pending_for(A, &[B]), vec![]);
    }

    #[test]
    fn leaving_and_re_entering_aoi_re_sends() {
        let mut r = AppearanceRelay::new();
        r.set(B, spec(1));
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(1))]);
        // B sort de l'AoI de A.
        assert_eq!(r.pending_for(A, &[]), vec![]);
        // B revient -> renvoi (auto-cicatrisant).
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(1))]);
    }

    #[test]
    fn an_observer_never_receives_its_own_appearance() {
        let mut r = AppearanceRelay::new();
        r.set(A, spec(1));
        r.set(B, spec(2));
        assert_eq!(r.pending_for(A, &[A, B]), vec![(B, spec(2))]);
    }

    #[test]
    fn a_subject_without_a_spec_is_not_sent() {
        let mut r = AppearanceRelay::new();
        // B est en AoI mais n'a pas encore choisi d'apparence.
        assert_eq!(r.pending_for(A, &[B]), vec![]);
        // Une fois posée, il est livré.
        r.set(B, spec(5));
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(5))]);
    }

    #[test]
    fn removing_a_subject_forgets_it_everywhere() {
        let mut r = AppearanceRelay::new();
        r.set(B, spec(1));
        assert_eq!(r.pending_for(A, &[B]), vec![(B, spec(1))]);
        r.remove(B);
        assert_eq!(r.get(B), None);
        // Même en AoI, plus rien à envoyer (déconnecté).
        assert_eq!(r.pending_for(A, &[B]), vec![]);
    }
}
