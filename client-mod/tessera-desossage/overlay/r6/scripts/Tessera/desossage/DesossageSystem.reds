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

  // Applique tous les leviers selon la config.
  public func Apply(game: GameInstance) -> Void {
    FTLog(s"[Tessera/Desossage] application des leviers…");
    this.ApplyPopulation(game);
    this.ApplyOrder(game);
    this.ApplyDevices(game);
    this.ApplyEvents(game);
    this.ApplyWorld(game);
    this.ApplyMappins(game);
    FTLog(s"[Tessera/Desossage] leviers appliqués.");
  }

  // Groupes — délèguent aux fonctions de leviers (corps réels à pincer en jeu, par lot).
  public func ApplyPopulation(game: GameInstance) -> Void {
    let c = this.m_config;
    Tessera_ApplyPedestrians(game, c.pedestrians);
    Tessera_ApplyTraffic(game, c.traffic);
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
  }

  public func ApplyWorld(game: GameInstance) -> Void {
    Tessera_ApplyDayNightScale(game, this.m_config.dayNightCycleScale);
  }

  public func ApplyMappins(game: GameInstance) -> Void {
    Tessera_ApplyMapMarkers(game, this.m_config.mapMarkers);
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
