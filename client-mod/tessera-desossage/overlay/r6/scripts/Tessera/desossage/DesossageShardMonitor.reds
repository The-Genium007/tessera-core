module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// Pont HUD moniteur de cohérence de shard (spec 2026-07-18-hud-moniteur-coherence-shard.md,
// Phase C Task 6/7). Le HUD Lua (TesseraHUD/init.lua) appelle ces méthodes sur le joueur pour
// comparer son calcul local (shard-map.json) au placement AUTORITAIRE poussé par le serveur.
//
// GATÉ (2026-07-18) — STUB TEMPORAIRE. Les corps délèguent normalement à
// GameInstance.GetNetworkGameSystem() (netcode C++ du fork). MAIS le type natif `NetworkGameSystem`
// n'est PAS résolu par le compilateur redscript (scc) au lancement : bien que le .dll netcode-v0.1.5
// enregistre la classe ET l'accesseur (symboles présents dans le binaire, vérifiés), scc échoue en
// [UNRESOLVED_TYPE] sur `GameInstance.GetNetworkGameSystem()`. C'est un bug de VISIBILITÉ RTTI de la
// registration RedLib du fork (les types ne sont pas visibles au moment où scc compile), DISTINCT
// de l'accesseur manquant (celui-là a bien été ajouté en netcode-v0.1.5). Comme redscript refuse
// TOUT r6/scripts sur une seule erreur, ce fichier faisait tomber le modset ENTIER au lancement via
// le launcher (jeu injouable).
//
// En attendant le vrai fix netcode (timing de registration RedLib à corriger dans le fork —
// diagnostiquer d'abord avec RedHotTools : NetworkGameSystem est-il dans le RTTI au runtime ?), on
// STUB : les 3 wrappers retournent la valeur neutre, comme s'il n'y avait aucune donnée serveur. Le
// HUD retombe en local-only — exactement le comportement de repli déjà prévu quand le système est
// null. On garde les @addMethod pour que l'appel Lua `player:Tessera_GetServerShard()` reste valide
// (pas d'erreur runtime côté CET/HUD).
//
// À RESTAURER (revert de ce stub) une fois que scc résout `NetworkGameSystem`, chaque corps redevient :
//   let net = GameInstance.GetNetworkGameSystem();
//   if !IsDefined(net) { return ""; }   // (ou 0 pour VisiblePlayerCount)
//   return net.Tessera_GetServerShard(); // (Overlaps / VisiblePlayerCount selon le wrapper)
// ─────────────────────────────────────────────────────────────────────────────

@addMethod(PlayerPuppet)
public func Tessera_GetServerShard() -> String {
  return "";
}

@addMethod(PlayerPuppet)
public func Tessera_GetServerOverlaps() -> String {
  return "";
}

@addMethod(PlayerPuppet)
public func Tessera_GetVisiblePlayerCount() -> Int32 {
  return 0;
}
