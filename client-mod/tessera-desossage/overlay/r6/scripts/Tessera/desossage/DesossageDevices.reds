module Tessera.Desossage

// Leviers dispositifs monde : voyage rapide (kiosques), vendeurs, distributeurs/interactables.
// STUB : journalise l'intention. Corps réel (FastTravelSystem, interactions devices) à pincer en jeu.

public func Tessera_ApplyFastTravel(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // Helper natif du FastTravelSystem : verrouille le voyage rapide (kiosques/dataterms inactifs).
  // Vérifié contre le script décompilé réel (CDPR-Modding-Documentation/Cyberpunk-Scripts,
  // scripts/core/systems/fastTravelSystem.script:850) — la citation précédente (« .swift »,
  // même défaut que le bug police) était bidon mais le symbole s'avère correct.
  FastTravelSystem.ManageFastTravelLock(false, n"tessera_desossage", game);
  FTLog(s"[Tessera/Desossage] voyage rapide → coupé (ManageFastTravelLock false)");
}

// Recherche (script décompilé officiel) : VendorComponent n'a que des getters de données
// (GetVendorID, GetJunkItemIDs...), aucun toggle actif/inactif. Les vendeurs AMBIANTS tombent
// probablement avec `pedestrians` (CommunitySystem.ChangeDensityModifier, cf. DesossagePopulation)
// s'ils sont spawnés comme PNJ communautaires — à tester en jeu. Les vendeurs NOMMÉS (fixes,
// scénarisés) restent un stub : pas de symbole de désactivation trouvé.
//
// Recherche élargie (2026-07-03, dump RTTI complet) : aucune classe "Shop"/"Store"/"Customer"
// pertinente. Cherché aussi un levier GÉNÉRIQUE (masquer/despawn n'importe quel PNJ, peu importe
// son rôle) qui aurait couvert vendeurs ET clients en magasin d'un coup : `entEntity`/
// `ScriptedPuppet` n'exposent aucune méthode visibilité/despawn/hide/destroy ;
// `SmartDespawnRequest`/`MarkDespawnCandidate` existent (événements internes du jeu, mécanisme de
// nettoyage de PNJ) mais n'ont aucun champ propre exposé — pas de point d'accroche scriptable
// trouvé pour les déclencher nous-mêmes. Piste générique épuisée côté RTTI.
// HYPOTHÈSE (à tester en jeu, pas encore fait) : les PNJ ambiants EN INTÉRIEUR (clients dans un
// magasin) sont probablement, comme le trafic, spawnés via le même mécanisme que `pedestrians`
// (zones `communityArea`/`worldCompiledCommunityAreaNode` vues dans le RTTI), donc potentiellement
// déjà coupés par le levier `pedestrians` existant, sans code supplémentaire.
//
// TROU COMBLÉ (2026-07-07) pour les vendeurs NOMMÉS (Wakako, Bill...) : présence physique du PNJ
// toujours indéboulonnable (aucune piste), mais leur COMMERCE est maintenant coupé par le hook
// `MenuScenario_Vendor.OnEnterScenario` ci-dessous — recherche déléguée (Fable 5, 2026-07-07),
// confirmée via le script décompilé officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts,
// scripts/cyberpunk/UI/fullscreen/vendor/vendorScenario.script) : c'est le point d'entrée UNIQUE
// de tout le commerce (vendor hub, ripperdoc, craft compris).
public func Tessera_ApplyVendors(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    FTLog(s"[Tessera/Desossage] vendeurs → normaux");
    return;
  }
  FTLog(s"[Tessera/Desossage] vendeurs → icône masquée (GameplayRoleComponent) + commerce bloqué (MenuScenario_Vendor.OnEnterScenario)");
}

// Point d'entrée unique de l'UI marchande (vendor hub, ripperdoc, crafting inclus) — un seul hook
// neutralise tous les vendeurs nommés d'un coup. Le PNJ reste visible, le commerce ne s'ouvre
// jamais : on redirige vers `GotoIdleState()` (méthode déjà déclarée sur la classe, gère
// proprement le retour en scénario idle) plutôt que de laisser un menu vide ouvert.
// PIN IN-GAME : jamais testé — risque identifié par la recherche (Fable 5, 2026-07-07) : la scène
// de dialogue qui a mené à ce scénario attend peut-être un événement de fermeture de menu
// spécifique ; si `GotoIdleState()` seul ne suffit pas, il faudra investiguer plus loin en jeu.
// Déclaré `cb func ... -> Bool` (pas `event ... -> Void`) : redscript 0.5.31 n'annote pas un
// `event`, et le RTTI donne `OnEnterScenario(CName, handle:IScriptable) -> Bool`.
@wrapMethod(MenuScenario_Vendor)
protected cb func OnEnterScenario(prevScenario: CName, userData: ref<IScriptable>) -> Bool {
  if !DesossageSystem.GetLiveConfig(GetGameInstance()).vendors.active {
    this.GotoIdleState();
    return true;
  }
  return wrappedMethod(prevScenario, userData);
}

public func Tessera_ApplyWorldDevices(game: GameInstance, vending: ref<DesossageEntry>, inter: ref<DesossageEntry>) -> Void {
  if !vending.active {
    // Le vrai coupe-circuit vit dans les 3 @wrapMethod GetActions ci-dessous (hooks partagés,
    // s'appliquent à toutes les instances) : VendingMachineControllerPS (boissons/nourriture,
    // confirmé), WeaponVendingMachineControllerPS et DropPointControllerPS (ajoutés 2026-07-07,
    // même famille de fix, PIN IN-GAME — classes PS sœurs, chacune avec son propre override de
    // GetActions comme VendingMachineControllerPS, donc pas couvertes par le hook générique
    // ScriptableDeviceComponentPS ci-dessous).
    FTLog(s"[Tessera/Desossage] distributeurs (boissons/nourriture/armes) + droppoints → coupés (GetActions)");
  }
  if !inter.active {
    // Le vrai coupe-circuit vit dans le @wrapMethod(ScriptableDeviceComponentPS) GetActions
    // ci-dessous — couvre les points d'accès/hackables ambiants (et toute autre PS de device qui
    // n'override pas GetActions elle-même, cf. note ci-dessous).
    FTLog(s"[Tessera/Desossage] interactables monde (points d'accès) → gérés par hook ScriptableDeviceComponentPS.GetActions");
  }
}

// CORRIGÉ (2026-07-05) — historique : `@wrapMethod(AccessPointControllerPS) GetActions(...)`
// échouait avec `[UNRESOLVED_METHOD] no method with this name exists on the target type`.
// Root cause identifiée via nativedb : contrairement à `VendingMachineControllerPS`/
// `SecurityTurretControllerPS` (cf. DesossageOrder.reds), qui déclarent chacune leur PROPRE
// override de `GetActions` (visible dans `search.py show <Classe>`, sans --deep),
// `AccessPointControllerPS` n'a AUCUN override propre — `GetActions` n'existe que sur
// `ScriptableDeviceComponentPS` (l'ancêtre commun), jamais réintroduit le long de la chaîne
// AccessPointControllerPS → MasterControllerPS → ScriptableDeviceComponentPS. `@wrapMethod`
// exige une méthode explicitement déclarée sur la classe ciblée, pas juste héritée.
// FIX : wrapper `ScriptableDeviceComponentPS.GetActions` directement (confirmé décl. propre via
// `search.py show ScriptableDeviceComponentPS` sans --deep) — couvre AccessPointControllerPS et
// toute autre PS de device qui n'override pas GetActions elle-même, sans affecter
// VendingMachineControllerPS/SecurityTurretControllerPS (dispatch virtuel : leur propre override
// prend le dessus, ce wrap de base ne s'exécute que pour les classes qui n'en ont pas).
// PIN IN-GAME : jamais testé — à confirmer (menu de hack absent sur un point d'accès ambiant).
// EXEMPTION ASCENSEURS (2026-07-27) — corrige le bloquant playtest « les boutons d'appel
// EXTÉRIEURS des ascenseurs ne répondent plus » (BACKLOG-INGAME §B2).
//
// Mécanisme exact, vérifié contre le script décompilé officiel (pas déduit) :
//   ElevatorFloorTerminalControllerPS extends TerminalControllerPS extends MasterControllerPS
//     -> ...GetActions termine par `return super.GetActions(outActions, context)`
//        (elevatorFloorTerminalController.script:225-241)
//   TerminalControllerPS.GetActions COMMENCE par
//        `if( !( super.GetActions( outActions, context ) ) ) { return false; }`
//        (terminalController.script) — il CONSOMME la valeur de retour.
// Donc notre `return false` ci-dessous ne se contente pas de sauter les actions de base : il
// court-circuite toute la chaîne terminal (ActionDeviceStatus, ToggleON, SetActionIllegality).
// Le panneau INTÉRIEUR y échappe parce que `LiftControllerPS.GetActions` est un override
// complet qui n'appelle jamais `super` — d'où le symptôme trompeur « ça marche de l'intérieur ».
// (Note : les appelants de haut niveau, eux, IGNORENT le booléen — `GetActions(actions, ctx);`
// en instruction. C'est bien l'étage TerminalControllerPS qui porte la panne, pas l'UI.)
//
// STATUT : contournement assumé, PAS la correction de fond. La décision du 2026-07-21 (« ne pas
// rustiner, reconstruire le désossage sur la doctrine d'interception ») reste valide — c'est le
// chantier C1. Cette exemption existe pour qu'un playtest lancé d'ici là ne soit pas cassé, et
// elle disparaîtra avec la reconstruction. Elle est volontairement étroite : une seule classe,
// aucun changement de comportement pour tout le reste des interactables monde.
// PIN IN-GAME : compilé (scc, 2026-07-27), effet NON confirmé en jeu — à vérifier sur un
// bouton d'appel extérieur, désossage actif avec `worldInteractables` coupé.
@wrapMethod(ScriptableDeviceComponentPS)
protected func GetActions(out actions: array<ref<DeviceAction>>, context: GetActionsContext) -> Bool {
  if IsDefined(this as ElevatorFloorTerminalControllerPS) {
    return wrappedMethod(actions, context);
  }
  if !DesossageSystem.GetLiveConfig(GetGameInstance()).worldInteractables.active {
    return false;
  }
  return wrappedMethod(actions, context);
}

// Coupe-circuit partagé pour les distributeurs boissons/nourriture (toutes instances, un seul
// hook). Signature moderne vérifiée contre un mod publié réel qui wrap la même classe PS
// (rfuzzo/cyberpunk-nexus-script-dump, mod 6927 « Enhanced Vending Machines »,
// _SharedDependencies.reds:380 — `@wrapMethod(VendingMachineControllerPS) ... GetQuickHackActions
// (out outActions:array<ref<DeviceAction>>, context:GetActionsContext)`) ; la méthode elle-même
// (GetActions, pas GetQuickHackActions) et son rôle sont confirmés par le script décompilé
// officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts, vendingMachineController.script:108).
// CORRIGÉ (2026-07-05) : lisait `DesossageConfig.Default()` direct — bug confirmé en jeu, cf.
// `DesossageSystem.GetLiveConfig`. Utilise maintenant l'état réellement vivant du panneau.
@wrapMethod(VendingMachineControllerPS)
protected func GetActions(out actions: array<ref<DeviceAction>>, context: GetActionsContext) -> Bool {
  if !DesossageSystem.GetLiveConfig(GetGameInstance()).vendingDevices.active {
    return false;
  }
  return wrappedMethod(actions, context);
}

// Distributeurs d'armes : PAS de hook propre. WeaponVendingMachineControllerPS hérite de
// VendingMachineControllerPS (vérifié au dump RTTI, 2026-07-15) et n'override PAS GetActions —
// le @wrapMethod qui vivait ici échouait donc en [UNRESOLVED_METHOD] (`no method with this name
// exists on the target type`), exactement le piège documenté plus haut pour AccessPointControllerPS.
// Les distributeurs d'armes sont déjà couverts par le wrap de VendingMachineControllerPS ci-dessus
// (dispatch virtuel : la classe fille hérite du GetActions wrappé), donc même levier
// `vendingDevices` que voulu, sans code supplémentaire.

// Droppoints — GetActions déclaré en propre (vérifié RTTI), donc wrappable. Même levier.
// PIN IN-GAME : jamais testé — à confirmer (menu de dépôt absent sur un droppoint).
@wrapMethod(DropPointControllerPS)
protected func GetActions(out actions: array<ref<DeviceAction>>, context: GetActionsContext) -> Bool {
  if !DesossageSystem.GetLiveConfig(GetGameInstance()).vendingDevices.active {
    return false;
  }
  return wrappedMethod(actions, context);
}
