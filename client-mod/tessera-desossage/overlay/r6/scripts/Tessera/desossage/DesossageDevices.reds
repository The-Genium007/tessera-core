module Tessera.Desossage

// Leviers dispositifs monde : voyage rapide (kiosques), vendeurs, distributeurs/interactables.
// STUB : journalise l'intention. Corps réel (FastTravelSystem, interactions devices) à pincer en jeu.

public func Tessera_ApplyFastTravel(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // Helper natif du FastTravelSystem : verrouille le voyage rapide (kiosques/dataterms inactifs).
  // Cf. fastTravelSystem.swift (ManageFastTravelLock(enable, reason, game, opt statusEffectID)).
  FastTravelSystem.ManageFastTravelLock(false, n"tessera_desossage", game);
  FTLog(s"[Tessera/Desossage] voyage rapide → coupé (ManageFastTravelLock false)");
}

public func Tessera_ApplyVendors(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : neutraliser l'interaction vendeur/marchand (tous : ambiants + nommés).
  FTLog(s"[Tessera/Desossage] (stub) vendeurs → coupés");
}

public func Tessera_ApplyWorldDevices(game: GameInstance, vending: ref<DesossageEntry>, inter: ref<DesossageEntry>) -> Void {
  if !vending.active {
    // PIN IN-GAME : désactiver distributeurs (boissons/nourriture) et droppoints.
    FTLog(s"[Tessera/Desossage] (stub) distributeurs/droppoints → coupés");
  }
  if !inter.active {
    // PIN IN-GAME : désactiver ripperdocs / points d'accès / hackables ambiants.
    FTLog(s"[Tessera/Desossage] (stub) interactables monde → coupés");
  }
}
