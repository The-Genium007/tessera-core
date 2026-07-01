module Tessera.Desossage

// Leviers dispositifs monde : voyage rapide (kiosques), vendeurs, distributeurs/interactables.
// STUB : journalise l'intention. Corps réel (FastTravelSystem, interactions devices) à pincer en jeu.

public func Tessera_ApplyFastTravel(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : désactiver FastTravelSystem + rendre les kiosques/dataterms inactifs.
  FTLog(s"[Tessera/Desossage] (stub) voyage rapide → coupé (kiosques inactifs)");
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
