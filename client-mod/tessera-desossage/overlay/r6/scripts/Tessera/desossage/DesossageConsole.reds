module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// Levier de test : bascule un système de DesossageConfig SANS rebuild, depuis la console CET.
// Usage (console CET, touche par défaut ~) :
//   Game.GetPlayer():Tessera_SetLever("police", true, 0.0)
//   Game.GetPlayer():Tessera_SetLever("pedestrians", true, 0.3)
//   Game.GetPlayer():Tessera_SetLever("dayNightCycleScale", true, 2.0)
// Ne persiste pas entre les rechargements de session (repart de DesossageConfig.Default() à
// chaque OnGameAttached) — pratique pour itérer en jeu, pas pour un réglage permanent (ça reste
// DesossageConfig.reds + rebuild pour ça).
//
// Bug corrigé (échec de compilation dev12→dev13) : `name == "police"` sur des String échoue en
// compilation (NO_MATCHING_OVERLOAD — l'opérateur == n'a pas de surcharge String malgré ce que
// dit la doc communautaire). Remplacé par StrCmp(a, b) == 0, vérifié contre deux mods réels qui
// compilent (TDUniverse/Cyberverse — le dépôt dont notre netcode est forké — et
// psiberx/cp2077-equipment-ex).
// ─────────────────────────────────────────────────────────────────────────────

@addMethod(PlayerPuppet)
public func Tessera_SetLever(name: String, active: Bool, density: Float) -> Void {
  let sys = DesossageSystem.Get(this.GetGame());
  if !IsDefined(sys) {
    FTLog(s"[Tessera/Desossage] SetLever: système introuvable (charge une session d'abord)");
    return;
  }
  let c = sys.GetConfig();

  if StrCmp(name, "dayNightCycleScale") == 0 {
    c.dayNightCycleScale = density;
    FTLog(s"[Tessera/Desossage] SetLever dayNightCycleScale → \(density)");
    sys.Apply(this.GetGame());
    return;
  }

  let e: ref<DesossageEntry>;
  if StrCmp(name, "pedestrians") == 0 { e = c.pedestrians; }
  else if StrCmp(name, "traffic") == 0 { e = c.traffic; }
  else if StrCmp(name, "vendors") == 0 { e = c.vendors; }
  else if StrCmp(name, "transit") == 0 { e = c.transit; }
  else if StrCmp(name, "police") == 0 { e = c.police; }
  else if StrCmp(name, "ambientSecurity") == 0 { e = c.ambientSecurity; }
  else if StrCmp(name, "ncpdHustles") == 0 { e = c.ncpdHustles; }
  else if StrCmp(name, "randomEncounters") == 0 { e = c.randomEncounters; }
  else if StrCmp(name, "cyberpsychos") == 0 { e = c.cyberpsychos; }
  else if StrCmp(name, "fastTravel") == 0 { e = c.fastTravel; }
  else if StrCmp(name, "vendingDevices") == 0 { e = c.vendingDevices; }
  else if StrCmp(name, "worldInteractables") == 0 { e = c.worldInteractables; }
  else if StrCmp(name, "questTriggers") == 0 { e = c.questTriggers; }
  else if StrCmp(name, "tutorials") == 0 { e = c.tutorials; }
  else if StrCmp(name, "mapMarkers") == 0 { e = c.mapMarkers; }
  else {
    FTLog(s"[Tessera/Desossage] SetLever: nom de levier inconnu \"\(name)\"");
    return;
  }

  e.active = active;
  e.density = density;
  FTLog(s"[Tessera/Desossage] SetLever \(name) → active=\(active) density=\(density)");
  sys.Apply(this.GetGame());
}
