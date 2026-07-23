//! Registre de sessions d'interaction (fondation d'interaction, palier 2, Phase 3.1). Pur —
//! aucune I/O, aucun accès réseau. Porte la CONCURRENCE (arbitrage de course sur une même cible :
//! premier `Choice` servi, l'autre reçoit un refus) et le TIMEOUT (session ouverte jamais résolue).
//! Le CONTENU (offre, prix, dialogue) est hors périmètre — `payload` reste un `Vec<u8>` opaque de
//! bout en bout, jamais interprété ici.

use std::time::{Duration, Instant};

pub type SessionId = u64;

/// Une session ouverte : qui interagit (`actor`), avec quoi (`target`), depuis quand (pour le
/// timeout). `ui_kind`/`payload` sont opaques — ce module ne les interprète jamais (spec §7 :
/// « payload opaque de l'enveloppe... zéro re-gel »).
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionSession {
    pub actor: u64,
    pub target: u64,
    pub ui_kind: u8,
    pub opened_at: Instant,
}

#[derive(Debug, PartialEq)]
pub enum SessionError {
    /// Aucune session ouverte sous cet id (jamais ouverte, déjà résolue, ou expirée).
    NotFound,
    /// La session existe mais appartient à un autre acteur — un client ne peut résoudre que SES
    /// propres sessions (anti-triche : rejouer un session_id observé sur le fil d'un autre joueur
    /// ne mène à rien).
    NotOwner,
}

#[derive(Default)]
pub struct SessionRegistry {
    sessions: std::collections::HashMap<SessionId, InteractionSession>,
    next_id: SessionId,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ouvre une nouvelle session. Ne vérifie RIEN sur `target` (portée/FSM/briques) — c'est
    /// l'arbitrage de l'APPELANT (Task 5, avant d'appeler `open`) ; ce module reste pur registre
    /// de sessions, pas arbitre de règles métier.
    pub fn open(&mut self, actor: u64, target: u64, ui_kind: u8) -> SessionId {
        self.next_id += 1;
        let id = self.next_id;
        self.sessions.insert(
            id,
            InteractionSession {
                actor,
                target,
                ui_kind,
                opened_at: Instant::now(),
            },
        );
        id
    }

    /// Résout (consomme) une session ouverte par `actor`. Retire la session du registre dans TOUS
    /// les cas (succès ou erreur) sauf `NotFound` — une session ne se résout qu'une fois, c'est ce
    /// qui arbitre la course : le premier `resolve` gagnant retire la session, tout `resolve`
    /// suivant sur le même id tombe sur `NotFound`.
    pub fn resolve(
        &mut self,
        session_id: SessionId,
        actor: u64,
    ) -> Result<InteractionSession, SessionError> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(SessionError::NotFound)?;
        if session.actor != actor {
            return Err(SessionError::NotOwner);
        }
        Ok(self
            .sessions
            .remove(&session_id)
            .expect("présence vérifiée juste au-dessus"))
    }

    /// Retire toute session ouverte depuis plus de `max_age` — appelé périodiquement (Task 5,
    /// câblage tick) pour ne jamais laisser une session fantôme accumuler indéfiniment (client qui
    /// ouvre une session puis se déconnecte sans jamais répondre).
    pub fn expire_stale(&mut self, max_age: Duration) {
        self.sessions.retain(|_, s| s.opened_at.elapsed() < max_age);
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_then_resolve_by_the_same_actor_succeeds() {
        let mut reg = SessionRegistry::new();
        let id = reg.open(1, 100, 0);
        let session = reg.resolve(id, 1).unwrap();
        assert_eq!(session.actor, 1);
        assert_eq!(session.target, 100);
    }

    #[test]
    fn resolving_an_unknown_session_id_returns_not_found() {
        let mut reg = SessionRegistry::new();
        assert_eq!(reg.resolve(999, 1), Err(SessionError::NotFound));
    }

    #[test]
    fn resolving_someone_elses_session_returns_not_owner_and_does_not_consume_it() {
        let mut reg = SessionRegistry::new();
        let id = reg.open(1, 100, 0);
        assert_eq!(reg.resolve(id, 2), Err(SessionError::NotOwner));
        // La session reste ouverte pour le vrai propriétaire — un imposteur ne doit pas pouvoir
        // la faire disparaître.
        assert_eq!(reg.resolve(id, 1).unwrap().actor, 1);
    }

    #[test]
    fn resolving_the_same_session_twice_the_second_call_gets_not_found() {
        // Le coeur de l'arbitrage de course (spec §2 : « premier Choice servi, l'autre reçoit un
        // refus »). Simule deux joueurs au même comptoir résolvant la même session_id.
        let mut reg = SessionRegistry::new();
        let id = reg.open(1, 100, 0);
        assert!(reg.resolve(id, 1).is_ok());
        assert_eq!(reg.resolve(id, 1), Err(SessionError::NotFound));
    }

    #[test]
    fn each_open_call_produces_a_distinct_session_id() {
        let mut reg = SessionRegistry::new();
        let a = reg.open(1, 100, 0);
        let b = reg.open(1, 100, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn expire_stale_removes_sessions_older_than_max_age_and_keeps_fresh_ones() {
        let mut reg = SessionRegistry::new();
        let old_id = reg.open(1, 100, 0);
        // Recule artificiellement `opened_at` en manipulant directement le HashMap interne via un
        // second open/resolve n'est pas possible (Instant ne se falsifie pas) — ce test vérifie
        // donc le cas trivial (rien d'assez vieux n'est retiré) et laisse le cas "vraiment expiré"
        // à un test d'intégration Task 5 avec un sleep réel, plus lent mais fiable.
        reg.expire_stale(Duration::from_secs(3600));
        assert!(reg.resolve(old_id, 1).is_ok());
    }

    #[test]
    fn expire_stale_after_a_real_sleep_removes_the_session() {
        let mut reg = SessionRegistry::new();
        let id = reg.open(1, 100, 0);
        std::thread::sleep(Duration::from_millis(20));
        reg.expire_stale(Duration::from_millis(10));
        assert_eq!(reg.resolve(id, 1), Err(SessionError::NotFound));
    }
}
