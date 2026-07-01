module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// LA config centrale du désossage. Un champ par système du jeu.
// Défaut = monde vide (tout coupé). Pour rallumer/ajuster un système : éditer
// `DesossageConfig.Default()` ci-dessous (ex. cyberpsychos = DesossageEntry.New(true, 0.3)).
// C'est le SEUL endroit à toucher pour piloter ce qui vit dans la ville.
// ─────────────────────────────────────────────────────────────────────────────

// Un levier : le système est-il présent dans le monde, et à quelle intensité.
public class DesossageEntry {
  public let active: Bool;
  public let density: Float; // 0.0 .. 1.0 (ignoré pour les bascules binaires)

  public static func New(active: Bool, density: Float) -> ref<DesossageEntry> {
    let e = new DesossageEntry();
    e.active = active;
    e.density = density;
    return e;
  }
}

public class DesossageConfig {
  // A. Population vivante
  public let pedestrians: ref<DesossageEntry>;
  public let traffic: ref<DesossageEntry>;
  public let vendors: ref<DesossageEntry>;
  public let transit: ref<DesossageEntry>;
  // B. Ordre public & hostilité
  public let police: ref<DesossageEntry>;
  public let ambientSecurity: ref<DesossageEntry>;
  // C. Événements / rencontres ambiantes
  public let ncpdHustles: ref<DesossageEntry>;
  public let randomEncounters: ref<DesossageEntry>;
  public let cyberpsychos: ref<DesossageEntry>;
  // D. Dispositifs & interactables monde
  public let fastTravel: ref<DesossageEntry>;
  public let vendingDevices: ref<DesossageEntry>;
  public let worldInteractables: ref<DesossageEntry>;
  // E. Quêtes & progression
  public let questTriggers: ref<DesossageEntry>;
  public let tutorials: ref<DesossageEntry>;
  // F. Monde (gardé, réglable) — 1.0 normal ; 2.0 = jour/nuit 2x plus long ; 0.0 = figé
  public let dayNightCycleScale: Float;

  // Défaut : monde vide (tout coupé). Chaque entrée est une instance distincte
  // (pour pouvoir en rallumer une sans toucher aux autres).
  public static func Default() -> ref<DesossageConfig> {
    let c = new DesossageConfig();
    // A.
    c.pedestrians = DesossageEntry.New(false, 0.0);
    c.traffic = DesossageEntry.New(false, 0.0);
    c.vendors = DesossageEntry.New(false, 0.0);
    c.transit = DesossageEntry.New(false, 0.0);
    // B.
    c.police = DesossageEntry.New(false, 0.0);
    c.ambientSecurity = DesossageEntry.New(false, 0.0);
    // C.
    c.ncpdHustles = DesossageEntry.New(false, 0.0);
    c.randomEncounters = DesossageEntry.New(false, 0.0);
    c.cyberpsychos = DesossageEntry.New(false, 0.0);
    // D.
    c.fastTravel = DesossageEntry.New(false, 0.0);
    c.vendingDevices = DesossageEntry.New(false, 0.0);
    c.worldInteractables = DesossageEntry.New(false, 0.0);
    // E.
    c.questTriggers = DesossageEntry.New(false, 0.0);
    c.tutorials = DesossageEntry.New(false, 0.0);
    // F.
    c.dayNightCycleScale = 1.0;
    return c;
  }
}
