module Tessera.Desossage

// Levier monde : échelle du cycle jour/nuit (le monde statique/temps/météo/son est gardé).
// STUB : journalise l'intention. Corps réel (échelle de temps du cycle) à pincer en jeu.

// 1.0 = normal ; 2.0 = journée 2x plus longue ; 0.0 = cycle figé.
// Recherche (dump RTTI du jeu) : gameTimeSystem.SetTimeDilation(reason, dilation, ...) existe
// mais c'est un ralenti GLOBAL (façon bullet-time/Sandevistan) qui affecterait AUSSI le
// mouvement/combat du joueur — contraire à la contrainte de design (« le joueur... gardé »,
// seuls temps/météo doivent varier). Pas de multiplicateur dédié cycle-jour/nuit-seul trouvé
// dans l'API de gameTimeSystem. PIN IN-GAME : creuser weatherSystem ou un multiplicateur dédié.
public func Tessera_ApplyDayNightScale(game: GameInstance, scale: Float) -> Void {
  FTLog(s"[Tessera/Desossage] (stub) cycle jour/nuit → échelle \(scale)");
}

// Saut direct à une heure précise — `gameTimeSystem.SetGameTimeByHMS(h, m, s, reason)`, confirmé
// par le script décompilé officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts,
// scripts/core/systems/timeSystem.script) ET plusieurs mods publiés réels qui l'appellent
// (CyanideX/NovaCityTools, MaximiliumM/appearancemenumod, Avi6481/EasyTrainers — tous via
// `Game.GetTimeSystem():SetGameTimeByHMS(...)`). Contrairement à MappinSystem, `gameTimeSystem`
// GARDE son nom RTTI complet côté redscript (pas de préfixe à retirer ici).
// N'affecte QUE l'horloge/l'éclairage — pas de ralenti joueur/combat, contrairement à
// SetTimeDilation (raison pour laquelle dayNightCycleScale, lui, reste un stub).
// PIN IN-GAME : jamais testé sur cette machine avant ce build.
public func Tessera_DoJumpToTime(game: GameInstance, hour: Int32, minute: Int32) -> Void {
  GameInstance.GetTimeSystem(game).SetGameTimeByHMS(hour, minute, 0, n"tessera_desossage");
  FTLog(s"[Tessera/Desossage] heure → \(hour)h\(minute)");
}

// Consommation du message protocole `WorldState` (serveur → tous les clients, cf.
// `server/src/gateway.rs::encode_world_state` + `protocol.fbs`, ajouté 2026-07-07) — horloge/
// météo monde partagée. Réutilise `SetGameTimeByHMS` (ci-dessus) pour l'heure ; `weather` est
// passé tel quel à `SetWeather`, string opaque côté serveur (le nom de record TweakDB exact
// reste à confirmer en jeu — une chaîne invalide échoue proprement, `SetWeather` renvoie false,
// aucun risque de plantage).
//
// PÉRIMÈTRE : cette fonction est prête à être appelée, mais RIEN ne l'appelle encore dans ce
// dépôt. Le décodage FlatBuffers du `ServerEnvelope`/`ServerMsg::WorldState` reçu sur le réseau
// est fait par le netcode C++ (`NetworkGameSystem`, fork Cyberverse `The-Genium007/Cyberverse`,
// PAS dans ce monorepo) — c'est ce netcode qui doit, une fois le message décodé, appeler cette
// fonction avec les valeurs reçues (pont C++ → redscript à câbler côté fork, chantier séparé,
// cf. `client-mod/INTEGRATION-server-contract.md`).
// PIN IN-GAME : jamais testé (dépend du câblage réseau ci-dessus, pas encore fait).
public func Tessera_ApplyWorldState(game: GameInstance, hour: Int32, minute: Int32, weather: String) -> Void {
  GameInstance.GetTimeSystem(game).SetGameTimeByHMS(hour, minute, 0, n"tessera_world_state");
  // PIN IN-GAME : `StringToName` est un helper redscript standard (String → CName), pas vérifié
  // spécifiquement pour ce fichier — si la compilation cloud le rejette, c'est sans risque
  // (erreur détectée en CI) : remplacer par l'équivalent confirmé (ex. `n"\(weather)"` si
  // l'interpolation de CName littéral s'avère insuffisante).
  let applied = GameInstance.GetWeatherSystem(game).SetWeather(StringToName(weather), 5.0, 1u);
  FTLog(s"[Tessera/Desossage] horloge monde → \(hour)h\(minute), météo \(weather) (appliquée=\(applied))");
}

// Lecture de l'heure locale du client — moitié « émission » du diagnostic de dérive playtest
// (protocole `ClientTimeReport`, cf. `server/src/gateway_routing.rs::extract_time_report` +
// `world_clock.rs`, 2026-07-07). Renvoie [heure, minute, seconde] tels qu'observés localement,
// à comparer côté serveur à sa propre horloge autoritaire.
// PIN IN-GAME : `GameTime.Hour()/Minute()/Sec()` (via `GetTimeSystem(game).GetGameTime()`) est
// l'API standard de lecture de l'heure, pas vérifiée spécifiquement pour ce fichier — source de
// confiance modérée (usage courant dans l'écosystème redscript CP77), à confirmer en jeu.
//
// PÉRIMÈTRE — même limite que `Tessera_ApplyWorldState` ci-dessus, dans l'autre sens : cette
// fonction LIT la donnée et la retourne, prête à être empaquetée en `ClientTimeReport` et
// envoyée — mais l'ENCODAGE FlatBuffers + l'ENVOI réseau vivent dans le netcode C++ (fork
// Cyberverse, PAS dans ce monorepo). Rien n'appelle cette fonction pour l'instant.
public func Tessera_ReadLocalTime(game: GameInstance) -> array<Int32> {
  let t = GameInstance.GetTimeSystem(game).GetGameTime();
  let out: array<Int32>;
  ArrayPush(out, t.Hour());
  ArrayPush(out, t.Minute());
  ArrayPush(out, t.Sec());
  return out;
}
