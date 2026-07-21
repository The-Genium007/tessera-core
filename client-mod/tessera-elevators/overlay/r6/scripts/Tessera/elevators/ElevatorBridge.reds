module Tessera.Elevators

// ─────────────────────────────────────────────────────────────────────────────────────────────
// ASCENSEURS — interception client (ADR 0012)
//
// Doctrine : « intercepter, pas détruire ». On ne réimplémente pas le mouvement de cabine (natif,
// piloté par une courbe d'asset illisible) — on redirige son DÉCLENCHEUR et on rejoue l'ordre
// serveur. Le moteur fournit alors gratuitement portes, sons, animation et portage du joueur.
//
// ÉTAPE 1 (2026-07-21) — observation seule : VALIDÉE EN JEU. Les trois hooks compilent et se
// déclenchent aux bons moments. Journal relevé :
//     GoToFloor  etage_courant=0 cible_avant=-1
//     GoToFloor  -> cible_apres=1  notification=SendThisEventToEntity
//     DEPART     vers_etage=1
// Et trois `CallElevator` d'affilée avec `cible=-1` quand la cabine est déjà à l'étage appelé —
// le jeu enregistre l'appel et ne déclenche aucun départ. Utile à savoir : un appel sans effet
// est NORMAL, ce n'est pas un hook muet.
//
// ÉTAPE 2 (ce fichier) — la COUPURE, réversible.
//   * mode OBSERVE (défaut) : rien ne change, on journalise. Le jeu se comporte comme en solo.
//   * mode INTERCEPT        : la boucle locale est coupée. `OnGoToFloor`/`OnCallElevator`
//                             retournent `DoNotNotifyEntity` sans appeler la méthode d'origine —
//                             l'entité n'est jamais notifiée, la cabine ne bouge pas. Et
//                             `SendLiftStartDelayedEvent` ne laisse passer QUE ce qui a été
//                             explicitement autorisé.
//
// Le mode se bascule à chaud depuis le harness ; rien n'est figé dans le binaire. C'est
// volontaire : une coupure qu'on ne peut pas annuler en jeu est une coupure qu'on ne peut pas
// diagnostiquer.
//
// SIGNATURES VÉRIFIÉES dans le script décompilé CDPR (jamais devinées — règle du protocole) :
//   liftController.script:1020  public export function OnGoToFloor( evt : GoToFloor ) : EntityNotificationType
//   liftController.script:1046  public export function OnCallElevator( evt : CallElevator ) : EntityNotificationType
//   lift.script:325             protected function SendLiftStartDelayedEvent( targetFloorIndex : Int32 )
// ─────────────────────────────────────────────────────────────────────────────────────────────

public func TesseraLiftLog(msg: String) -> Void {
  FTLog(s"[Tessera/Ascenseur] \(msg)");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// L'état de l'interception, porté par un système du jeu pour être joignable depuis le harness
// (Lua) comme depuis les hooks. Même motif que `DesossageSystem`, qui a fait ses preuves ici.
// ─────────────────────────────────────────────────────────────────────────────────────────────
public class ElevatorBridgeSystem extends ScriptableSystem {
  // false = OBSERVE (le jeu se comporte normalement). Défaut volontaire : on ne coupe jamais
  // un comportement de jeu sans que quelqu'un l'ait explicitement demandé.
  private let m_intercept: Bool;

  // Étage dont le départ est autorisé, ou -1. Autorisation À USAGE UNIQUE : consommée au premier
  // départ. Sinon un ordre serveur unique laisserait la porte ouverte à tous les départs suivants,
  // y compris ceux qu'on veut justement empêcher (reprise après streaming, rattrapage).
  private let m_authorizedFloor: Int32;

  private func OnAttach() -> Void {
    this.m_intercept = false;
    this.m_authorizedFloor = -1;
    TesseraLiftLog("systeme attache — mode OBSERVE (aucune coupure)");
  }

  public static func Get(game: GameInstance) -> ref<ElevatorBridgeSystem> {
    let container = GameInstance.GetScriptableSystemsContainer(game);
    return container.Get(n"Tessera.Elevators.ElevatorBridgeSystem") as ElevatorBridgeSystem;
  }

  public func SetIntercept(on: Bool) -> Void {
    this.m_intercept = on;
    this.m_authorizedFloor = -1;
    TesseraLiftLog(s"mode -> \(on ? "INTERCEPT (cabines coupees)" : "OBSERVE (comportement solo)")");
  }

  public func IsIntercepting() -> Bool {
    return this.m_intercept;
  }

  // Autorise UN départ vers `floor`. C'est ce que le serveur appellera plus tard ; pour l'instant
  // le harness s'en charge, ce qui permet de valider la mécanique sans réseau.
  public func AuthorizeDeparture(floor: Int32) -> Void {
    this.m_authorizedFloor = floor;
    TesseraLiftLog(s"depart AUTORISE vers etage \(floor)");
  }

  // Consomme l'autorisation. Retourne true si ce départ précis était autorisé.
  public func ConsumeAuthorization(floor: Int32) -> Bool {
    if this.m_authorizedFloor == floor {
      this.m_authorizedFloor = -1;
      return true;
    }
    return false;
  }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 1) SÉLECTION D'ÉTAGE depuis l'intérieur de la cabine.
//
// En interception : on retourne `DoNotNotifyEntity` SANS appeler la méthode d'origine. L'entité
// n'est jamais notifiée, `m_targetFloor` n'est pas écrit, la cabine ne bouge pas. C'est la coupure
// la plus propre — elle laisse tout l'appareillage vanilla intact en aval, prêt à être rejoué.
// ─────────────────────────────────────────────────────────────────────────────────────────────
@wrapMethod(LiftControllerPS)
public func OnGoToFloor(evt: ref<GoToFloor>) -> EntityNotificationType {
  let sys = ElevatorBridgeSystem.Get(GetGameInstance());
  if IsDefined(sys) && sys.IsIntercepting() {
    TesseraLiftLog(s"GoToFloor  COUPE (etage_courant=\(this.GetActiveFloor()))");
    return EntityNotificationType.DoNotNotifyEntity;
  }
  TesseraLiftLog(s"GoToFloor  etage_courant=\(this.GetActiveFloor()) cible_avant=\(this.GetTargetFloor())");
  let res = wrappedMethod(evt);
  TesseraLiftLog(s"GoToFloor  -> cible_apres=\(this.GetTargetFloor())  notification=\(ToString(res))");
  return res;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 2) APPEL depuis un palier. Chemin distinct : le terminal d'étage remonte au maître par
// `GetParents()`. Vérifié en jeu — on peut commander une cabine où AUCUN joueur ne se trouve,
// ce qui est le cas multijoueur central.
// ─────────────────────────────────────────────────────────────────────────────────────────────
@wrapMethod(LiftControllerPS)
public func OnCallElevator(evt: ref<CallElevator>) -> EntityNotificationType {
  let sys = ElevatorBridgeSystem.Get(GetGameInstance());
  if IsDefined(sys) && sys.IsIntercepting() {
    TesseraLiftLog(s"CallElevator  COUPE (etage_courant=\(this.GetActiveFloor()))");
    return EntityNotificationType.DoNotNotifyEntity;
  }
  TesseraLiftLog(s"CallElevator  etage_courant=\(this.GetActiveFloor()) en_mouvement=\(this.IsMoving())");
  let res = wrappedMethod(evt);
  TesseraLiftLog(s"CallElevator  -> cible=\(this.GetTargetFloor())  notification=\(ToString(res))");
  return res;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 3) L'ENTONNOIR UNIQUE DE DÉPART — le filet de sécurité.
//
// TOUTE mise en route passe ici : action joueur, nœud de quête, reprise après streaming, et le
// rattrapage `VerifyDestination`. Bloquer aux deux points précédents ne suffit donc PAS : sans ce
// troisième garde, une cabine pourrait repartir seule après un stream-in, hors de tout contrôle.
//
// Le rejeu serveur passe lui aussi par ici (`QuestForceGoToFloor` y aboutit) — d'où l'autorisation
// à usage unique plutôt qu'un simple interrupteur.
// ─────────────────────────────────────────────────────────────────────────────────────────────
@wrapMethod(LiftDevice)
protected func SendLiftStartDelayedEvent(targetFloorIndex: Int32) -> Void {
  let sys = ElevatorBridgeSystem.Get(GetGameInstance());
  if IsDefined(sys) && sys.IsIntercepting() {
    if !sys.ConsumeAuthorization(targetFloorIndex) {
      TesseraLiftLog(s"DEPART  BLOQUE vers_etage=\(targetFloorIndex) (non autorise)");
      return;
    }
    TesseraLiftLog(s"DEPART  REJOUE vers_etage=\(targetFloorIndex) (ordre autorise)");
  } else {
    TesseraLiftLog(s"DEPART  vers_etage=\(targetFloorIndex)");
  }
  wrappedMethod(targetFloorIndex);
}
