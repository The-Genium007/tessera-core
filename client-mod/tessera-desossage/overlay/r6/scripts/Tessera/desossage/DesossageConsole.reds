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
// ─────────────────────────────────────────────────────────────────────────────

@addMethod(PlayerPuppet)
public func Tessera_SetLever(name: String, active: Bool, density: Float) -> Void {
  let sys = DesossageSystem.Get(this.GetGame());
  if !IsDefined(sys) {
    FTLog(s"[Tessera/Desossage] SetLever: système introuvable (charge une session d'abord)");
    return;
  }
  let c = sys.GetConfig();

  if name == "dayNightCycleScale" {
    c.dayNightCycleScale = density;
    FTLog(s"[Tessera/Desossage] SetLever dayNightCycleScale → \(density)");
    sys.Apply(this.GetGame());
    return;
  }

  let e: ref<DesossageEntry>;
  if name == "pedestrians" { e = c.pedestrians; }
  else if name == "traffic" { e = c.traffic; }
  else if name == "vendors" { e = c.vendors; }
  else if name == "transit" { e = c.transit; }
  else if name == "police" { e = c.police; }
  else if name == "ambientSecurity" { e = c.ambientSecurity; }
  else if name == "ncpdHustles" { e = c.ncpdHustles; }
  else if name == "randomEncounters" { e = c.randomEncounters; }
  else if name == "cyberpsychos" { e = c.cyberpsychos; }
  else if name == "fastTravel" { e = c.fastTravel; }
  else if name == "vendingDevices" { e = c.vendingDevices; }
  else if name == "worldInteractables" { e = c.worldInteractables; }
  else if name == "questTriggers" { e = c.questTriggers; }
  else if name == "tutorials" { e = c.tutorials; }
  else {
    FTLog(s"[Tessera/Desossage] SetLever: nom de levier inconnu \"\(name)\"");
    return;
  }

  e.active = active;
  e.density = density;
  FTLog(s"[Tessera/Desossage] SetLever \(name) → active=\(active) density=\(density)");
  sys.Apply(this.GetGame());
}
