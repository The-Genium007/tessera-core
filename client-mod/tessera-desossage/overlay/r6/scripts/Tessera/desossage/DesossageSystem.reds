module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// L'applicateur du désossage. ScriptableSystem instancié par le jeu ; au chargement
// de la session (rattachement du joueur), il lit la config et applique chaque levier.
// Les corps réels des leviers vivent dans DesossagePopulation/Order/Devices/Events/World ;
// ici on orchestre. Idempotent : ré-appelable sur rechargement.
// ─────────────────────────────────────────────────────────────────────────────

public class DesossageSystem extends ScriptableSystem {
  private let m_config: ref<DesossageConfig>;

  private func OnAttach() -> Void {
    this.m_config = DesossageConfig.Default();
    FTLog(s"[Tessera/Desossage] système attaché — config chargée");
  }

  public func GetConfig() -> ref<DesossageConfig> {
    return this.m_config;
  }

  public static func Get(game: GameInstance) -> ref<DesossageSystem> {
    let container = GameInstance.GetScriptableSystemsContainer(game);
    return container.Get(n"Tessera.Desossage.DesossageSystem") as DesossageSystem;
  }

  // CORRIGE UN BUG CONFIRMÉ EN JEU (2026-07-05) : plusieurs coupe-circuits @wrapMethod
  // (worldInteractables, vendingDevices, police, ambientSecurity, vendors, gangHostility) lisaient
  // `DesossageConfig.Default()` directement — qui reconstruit un OBJET NEUF avec les valeurs par
  // défaut codées en dur à CHAQUE appel, jamais l'état réellement coché dans le panneau CET. Bug
  // masqué jusqu'ici car les tests précédents comparaient surtout contre l'état par défaut
  // (qui coïncide avec Default()). Révélé par `gangHostility` : fonctionnait seulement après un
  // cocher/décocher manuel (qui, via Tessera_SetLever → Apply(), pousse un effet de bord réel côté
  // AttitudeSystem — mais le wrapMethod lui-même ignorait toujours l'état affiché). Cette fonction
  // lit l'état RÉELLEMENT vivant (`sys.GetConfig()`) quand le système est attaché, et ne retombe
  // sur `Default()` que si `DesossageSystem` n'est pas encore attaché (tout début de boot).
  public static func GetLiveConfig(game: GameInstance) -> ref<DesossageConfig> {
    let sys = DesossageSystem.Get(game);
    if IsDefined(sys) {
      return sys.GetConfig();
    }
    return DesossageConfig.Default();
  }

  // Applique tous les leviers selon la config.
  public func Apply(game: GameInstance) -> Void {
    FTLog(s"[Tessera/Desossage] application des leviers…");
    this.ApplyPopulation(game);
    this.ApplyOrder(game);
    this.ApplyDevices(game);
    this.ApplyEvents(game);
    this.ApplyWorld(game);
    this.ApplyMappins(game);
    this.LogGateSnapshot();
    FTLog(s"[Tessera/Desossage] leviers appliqués.");
  }

  // Groupes — délèguent aux fonctions de leviers (corps réels à pincer en jeu, par lot).
  public func ApplyPopulation(game: GameInstance) -> Void {
    let c = this.m_config;
    Tessera_ApplyPedestrians(game, c.pedestrians);
    Tessera_ApplyTransit(game, c.transit);
  }

  public func ApplyOrder(game: GameInstance) -> Void {
    let c = this.m_config;
    Tessera_ApplyPolice(game, c.police);
    Tessera_ApplyAmbientSecurity(game, c.ambientSecurity);
    Tessera_ApplyGangHostility(game, c.gangHostility);
  }

  public func ApplyDevices(game: GameInstance) -> Void {
    let c = this.m_config;
    Tessera_ApplyFastTravel(game, c.fastTravel);
    Tessera_ApplyVendors(game, c.vendors);
    Tessera_ApplyWorldDevices(game, c.vendingDevices, c.worldInteractables);
  }

  public func ApplyEvents(game: GameInstance) -> Void {
    let c = this.m_config;
    Tessera_ApplyEncounterCategory(game, n"ncpdHustles", c.ncpdHustles);
    Tessera_ApplyEncounterCategory(game, n"randomEncounters", c.randomEncounters);
    Tessera_ApplyEncounterCategory(game, n"cyberpsychos", c.cyberpsychos);
    Tessera_ApplyQuestTriggers(game, c.questTriggers);
    Tessera_ApplyTutorials(game, c.tutorials);
    Tessera_ApplyAirTraffic(game, c.airTraffic);
  }

  public func ApplyWorld(game: GameInstance) -> Void {
    Tessera_ApplyDayNightScale(game, this.m_config.dayNightCycleScale);
  }

  public func ApplyMappins(game: GameInstance) -> Void {
    Tessera_ApplyMapMarkers(game, this.m_config.mapMarkers);
  }

  // Gate — canal état (2026-07-16) : dump de l'état RÉELLEMENT vivant de chaque levier (pas
  // DesossageConfig.Default(), qui ignore les changements faits via la console CET — même piège
  // déjà corrigé ailleurs dans ce fichier, cf. GetLiveConfig). Appelé à la fin de Apply() : donc
  // au chargement de session ET à chaque changement manuel (Tessera_SetLever rappelle Apply()).
  private func LogGateSnapshot() -> Void {
    let c = this.m_config;
    FTLog(s"[Tessera/Gate/State] pedestrians active=\(c.pedestrians.active) density=\(c.pedestrians.density)");
    FTLog(s"[Tessera/Gate/State] vendors active=\(c.vendors.active) density=\(c.vendors.density)");
    FTLog(s"[Tessera/Gate/State] transit active=\(c.transit.active) density=\(c.transit.density)");
    FTLog(s"[Tessera/Gate/State] police active=\(c.police.active) density=\(c.police.density)");
    FTLog(s"[Tessera/Gate/State] ambientSecurity active=\(c.ambientSecurity.active) density=\(c.ambientSecurity.density)");
    FTLog(s"[Tessera/Gate/State] gangHostility active=\(c.gangHostility.active) density=\(c.gangHostility.density)");
    FTLog(s"[Tessera/Gate/State] ncpdHustles active=\(c.ncpdHustles.active) density=\(c.ncpdHustles.density)");
    FTLog(s"[Tessera/Gate/State] randomEncounters active=\(c.randomEncounters.active) density=\(c.randomEncounters.density)");
    FTLog(s"[Tessera/Gate/State] cyberpsychos active=\(c.cyberpsychos.active) density=\(c.cyberpsychos.density)");
    FTLog(s"[Tessera/Gate/State] fastTravel active=\(c.fastTravel.active) density=\(c.fastTravel.density)");
    FTLog(s"[Tessera/Gate/State] vendingDevices active=\(c.vendingDevices.active) density=\(c.vendingDevices.density)");
    FTLog(s"[Tessera/Gate/State] worldInteractables active=\(c.worldInteractables.active) density=\(c.worldInteractables.density)");
    FTLog(s"[Tessera/Gate/State] questTriggers active=\(c.questTriggers.active) density=\(c.questTriggers.density)");
    FTLog(s"[Tessera/Gate/State] tutorials active=\(c.tutorials.active) density=\(c.tutorials.density)");
    FTLog(s"[Tessera/Gate/State] airTraffic active=\(c.airTraffic.active) density=\(c.airTraffic.density)");
    FTLog(s"[Tessera/Gate/State] mapMarkers active=\(c.mapMarkers.active) density=\(c.mapMarkers.density)");
    FTLog(s"[Tessera/Gate/State] dayNightCycleScale=\(c.dayNightCycleScale)");
  }
}

// Déclencheur : au rattachement du joueur (monde chargé), appliquer le désossage une fois.
// PIN IN-GAME : confirmer que `PlayerPuppet.OnGameAttached` est le point d'entrée fiable
// (sinon basculer certains leviers sur un DelaySystem court).
@wrapMethod(PlayerPuppet)
protected cb func OnGameAttached() -> Bool {
  let res = wrappedMethod();
  let sys = DesossageSystem.Get(this.GetGame());
  if IsDefined(sys) {
    sys.Apply(this.GetGame());
  } else {
    FTLog(s"[Tessera/Desossage] ERREUR : DesossageSystem introuvable au rattachement");
  }
  return res;
}
