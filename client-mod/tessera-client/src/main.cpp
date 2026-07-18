// Plugin RED4ext minimal « Tessera » — preuve de la chaîne de livraison client + désossage natif.
//
// À l'origine, ce plugin ne faisait RIEN (juste une ligne de log au chargement, pour valider la
// chaîne cloud-build Windows → emballage modset signé → install launcher → chargement en jeu,
// l'équivalent côté client du `probe` côté serveur). Le vrai port Cyberverse (GNS + FlatBuffers)
// reste un plugin/dépôt séparé, pas ici.
//
// Depuis 2026-07-07, ce plugin porte aussi le désossage natif (phase 1, log-only — voir le
// commentaire détaillé plus bas) : les leviers redscript (`tessera-desossage/`) ne peuvent pas
// atteindre certains spawns purement natifs, ce hook C++ observe le point d'accroche identifié
// avant d'envisager un blocage réel.
//
// API ciblée : RED4ext SDK 1.0.0 (API v1, namespace RED4ext::v1). Ce SDK cible le jeu 2.31.
// Déclaré RUNTIME_INDEPENDENT malgré le hook d'adresse précise ci-dessous — voir la note dans
// `Query()` sur pourquoi ce n'est pas encore épinglé à 2.31 spécifiquement.

#include <RED4ext/RED4ext.hpp>
#include <chrono>
#include <cstdint>
#include <string>
#include <unordered_map>

// ─────────────────────────────────────────────────────────────────────────────
// Désossage natif — PHASE 1 : hook LOG-ONLY, aucun blocage.
//
// 5 catégories (cyberpsychos, hustles NCPD, gigs/donneurs de quête par proximité, événements
// aléatoires, PNJ statiques "community") sont hors de portée du redscript — leur spawn est
// exécuté nativement via un nœud de graphe de quête. Recherche dédiée (Fable 5, 2026-07-07)
// identifie le point d'accroche : `QuestPhaseInstance::ExecuteNode`, le dispatcheur central de
// TOUS les nœuds de TOUTES les quêtes du jeu — recommandation explicite « GO PRUDENT » : d'abord
// observer en jeu (log seul, laisser tourner l'original sans jamais bloquer), avant d'envisager un
// blocage réel (phase 2, pas fait ici — la sémantique du retour/des sockets de sortie n'est pas
// encore comprise, un mauvais blocage pourrait geler des graphes de quête en aval).
//
// Hash AddressLib confirmé par 2 sources indépendantes : Codeware
// (psiberx/cp2077-codeware, src/Red/Addresses/Library.hpp,
// `QuestPhaseInstance_ExecuteNode = 3227858325`) et alphanin9/SharedPunk
// (src/include/Impl/Detail/Hashes.hpp, même valeur). Codeware APPELLE cette fonction en
// production (son QuestPhaseExecutor), ce qui valide la signature à grande échelle — mais
// personne ne la HOOKE publiquement, d'où la prudence phase 1.
//
// PIN IN-GAME : la signature exacte (types de `aContext`/sockets) vient du SDK généré
// (`Generated/quest/*.hpp`), pas vérifiée localement (pas d'accès Windows) — laissée en types
// opaques (`void*`) pour ne prendre aucun risque de mauvaise réinterprétation mémoire. Seul le
// paramètre qu'on inspecte (`aNode`) est typé. Si la compilation cloud échoue, ajuster la
// signature avant de retenter — un échec de compilation est sans risque (détecté en CI, jamais
// vu par un joueur), contrairement à une signature fausse qui compilerait quand même.
//
// PIN IN-GAME : si le hash `3227858325` est absent de `cyberpunk2077_addresses.json` (livré par
// CDPR dans `bin\x64\` depuis le patch 2.3, vérifiable via `grep -A2 3227858325
// cyberpunk2077_addresses.json` sur le PC de test AVANT tout déploiement), la résolution
// natif échoue bruyamment au chargement (boîte d'erreur RED4ext, processus terminé) — échec
// propre et déterministe, pas un crash aléatoire (comportement documenté du SDK,
// `UniversalRelocBase::Resolve`).
//
// GATE (2026-07-16) — élargi de « spawn nodes seulement » à « tous les nœuds vus » : le but n'est
// plus seulement de confirmer les nœuds spawn, mais de DÉCOUVRIR par leur nom de classe ce qui
// correspond à l'appel Takemura / aux PNJ statiques Community / aux hustles — inconnu à l'avance.
// Dédupliqué par nom de classe (3 premières occurrences en détail, puis comptage silencieux + un
// résumé tous les `kSummaryEveryNCalls` appels) pour ne pas noyer le log : `ExecuteNode` est le
// dispatcheur central, appelé en continu pour toutes les quêtes en cours. Toujours strictement
// log-only : `g_original(...)` est appelé inconditionnellement dans tous les cas, aucun retour
// anticipé, aucun changement de comportement.
namespace TesseraDesossageNative
{
// Signature RÉELLE, confirmée en jeu (crash 2026-07-18) puis calée sur Codeware
// (Red/QuestsSystem.hpp:286, RawFunc @ hash 3227858325) :
//   uint8_t ExecuteNode(questPhaseInstance* aPhase, questNodeDefinition* aInputNode,
//                       QuestContext& aContext, const QuestNodeSocket& aInputSocket,
//                       DynArray<QuestNodeSocket>& aOutputSockets)
// → 5 paramètres. La version précédente en déclarait 6 (avec un faux `CClass*` en 2e position et
// un `aNode` en trop) : convention d'appel corrompue → CRASH au 1er nœud exécuté (le hook
// s'attachait sans erreur mais mourait dès qu'un ExecuteNode firait ; invisible tant que redscript
// échouait avant, révélé une fois la compilation réparée). Sur x64 les 3 références (aContext /
// sockets) passent comme des pointeurs : `void*` pour chacune matche l'ABI, on ne les touche pas,
// on les repasse telles quelles à g_original. Seul aInputNode est inspecté (sa classe RTTI).
using ExecuteNode_t = std::uint8_t (*)(void* aPhase, void* aInputNode, void* aContext,
    void* aInputSocket, void* aOutputSockets);

constexpr std::uint32_t kExecuteNodeHash = 3227858325u;

ExecuteNode_t g_original = nullptr;
RED4ext::v1::Sdk const* g_sdk = nullptr;
RED4ext::v1::PluginHandle g_handle = nullptr;

// Nom RTTI de la classe de nœud spawn — englobe questSpawner_NodeType/questSpawnSet_NodeType/
// questCommunityTemplate_NodeType (tous héritent de questSpawnManagerNodeDefinition, confirmé
// via le SDK). Conservé pour marquer les nœuds spawn dans le log élargi ci-dessous.
constexpr const char* kSpawnNodeClassName = "questSpawnManagerNodeDefinition";

// PALIER 2 (2026-07-18) — blocage par PHASE de quête (path du .questphase), PAS par classe.
// Le test « bloquer questPhoneManagerNodeDefinition » a échoué (le blocage firait, jeu stable, mais
// Takemura appelait quand même : l'appel est une SCÈNE, pas le phone manager). Bonne nouvelle
// dérivée : le mécanisme de blocage marche sans casser le graphe. La bonne granularité, c'est la
// PHASE : un nœud `questPhaseNodeDefinition` porte `.phaseResource` = le path du .questphase de la
// sous-quête qu'il ENTRE. Bloquer ce nœud = TOUTE la sous-quête (quête/gig/hustle/appel) ne
// s'exécute jamais — même effet que les mods archive (override de .questphase) mais au RUNTIME,
// sans redistribuer d'asset CDPR. Le path est un hash uint64 (ResourcePath). Chaque hash = une
// quête précise ; un dossier (gigs/, minor_activities/…) = un TYPE ou un DONNEUR entier.
constexpr const char* kPhaseNodeClassName = "questPhaseNodeDefinition";
constexpr const char* kPhaseResourceProp = "phaseResource";

// Hashes de path .questphase à bloquer. VIDE (que la sentinelle 0) = étape 1 : observation seule,
// AUCUN blocage. On les remplit à l'étape 3, après avoir lu les hashes dans le log
// [Tessera/Gate/Phase] et mappé hash→quête en jeu (+ WolvenKit / paths connus des mods).
constexpr std::uint64_t kBlockedPhaseHashes[] = { 0u };

constexpr std::uint64_t kDetailedLogLimit = 3;
constexpr std::uint64_t kSummaryEveryNCalls = 500;

std::unordered_map<std::string, std::uint64_t> g_classSeenCount;
std::uint64_t g_totalCalls = 0;

std::uint64_t NowMillis()
{
    using namespace std::chrono;
    return static_cast<std::uint64_t>(
        duration_cast<milliseconds>(system_clock::now().time_since_epoch()).count());
}

std::uint8_t Detour(void* aPhase, void* aInputNode, void* aContext, void* aInputSocket,
    void* aOutputSockets)
{
    // PALIER 2 : les nœuds de PHASE (kPhaseNodeClassName) sont inspectés (log de leur hash de
    // .questphase) et bloqués SI leur hash est dans kBlockedPhaseHashes — vide pour l'instant, donc
    // étape 1 = observation pure. Tout le reste est log-only (g_original appelé en fin de fonction).
    ++g_totalCalls;
    if (aInputNode != nullptr && g_sdk != nullptr)
    {
        // aInputNode est un questNodeDefinition* (ISerializable) : sa classe RTTI vient de GetType(),
        // PAS d'un CClass* passé en argument (l'erreur d'origine qui lisait de la mémoire au hasard).
        auto* node = reinterpret_cast<RED4ext::ISerializable*>(aInputNode);
        auto* nodeClass = node->GetType();
        const char* className = (nodeClass != nullptr) ? nodeClass->name.ToString() : nullptr;
        std::string key(className != nullptr ? className : "<sans nom>");

        // PALIER 2 — identité de PHASE : un questPhaseNodeDefinition porte le path (.questphase) de
        // la sous-quête qu'il entre. On lit ce hash via la propriété RTTI phaseResource (raRef →
        // ResourcePath.hash à l'offset 0), on le LOGUE (étape 1 : observation), et on BLOQUE la phase
        // si son hash est dans kBlockedPhaseHashes (étape 3). Bloquer ce nœud = sous-quête neutralisée.
        if (key == kPhaseNodeClassName && nodeClass != nullptr)
        {
            std::uint64_t phaseHash = 0u;
            auto* prop = nodeClass->GetProperty(RED4ext::CName(kPhaseResourceProp));
            if (prop != nullptr)
            {
                auto* pathPtr = prop->GetValuePtr<RED4ext::ResourcePath>(node);
                if (pathPtr != nullptr) { phaseHash = pathPtr->hash; }
            }
            bool blocked = false;
            for (std::uint64_t h : kBlockedPhaseHashes)
            {
                if (h != 0u && h == phaseHash) { blocked = true; break; }
            }
            g_sdk->logger->InfoF(g_handle,
                "[Tessera/Gate/Phase] t=%llu seq=%llu phaseHash=0x%016llX%s",
                static_cast<unsigned long long>(NowMillis()),
                static_cast<unsigned long long>(g_totalCalls),
                static_cast<unsigned long long>(phaseHash),
                blocked ? " — BLOQUE (sous-quete neutralisee, g_original NON appele)" : "");
            if (blocked) { return 1; }
        }

        std::uint64_t& count = g_classSeenCount[key];
        ++count;

        if (count <= kDetailedLogLimit)
        {
            auto* rtti = RED4ext::CRTTISystem::Get();
            auto* spawnCls = rtti != nullptr ? rtti->GetClass(kSpawnNodeClassName) : nullptr;
            bool isSpawnNode = spawnCls != nullptr && nodeClass != nullptr && nodeClass->IsA(spawnCls);
            g_sdk->logger->InfoF(g_handle,
                "[Tessera/Gate/Node] t=%llu seq=%llu classe=%s occurrence=%llu spawnNode=%d — log seul, phase 1",
                static_cast<unsigned long long>(NowMillis()),
                static_cast<unsigned long long>(g_totalCalls), key.c_str(),
                static_cast<unsigned long long>(count), isSpawnNode ? 1 : 0);
        }

        if (g_totalCalls % kSummaryEveryNCalls == 0)
        {
            g_sdk->logger->InfoF(g_handle,
                "[Tessera/Gate/Summary] t=%llu total=%llu classesDistinctes=%zu",
                static_cast<unsigned long long>(NowMillis()),
                static_cast<unsigned long long>(g_totalCalls),
                static_cast<size_t>(g_classSeenCount.size()));
        }
    }
    return g_original(aPhase, aInputNode, aContext, aInputSocket, aOutputSockets);
}
} // namespace TesseraDesossageNative

// Query — le loader lit notre identité avant de nous charger.
RED4EXT_C_EXPORT void RED4EXT_CALL Query(RED4ext::v1::PluginInfo* aInfo)
{
    aInfo->name = L"TesseraClient";
    aInfo->author = L"TesseraSynth";
    aInfo->version = RED4EXT_V1_SEMVER(0, 1, 0);
    // Recommandation de la recherche (2026-07-07) : épingler à 2.31 maintenant qu'on hooke une
    // adresse précise (ExecuteNode), plutôt que INDEPENDENT. Laissé en INDEPENDENT ici : le nom
    // exact de la constante RED4EXT_V1_RUNTIME_VERSION_2_31 n'a pas pu être vérifié sans accès
    // local au SDK — un mauvais nom serait une erreur de compilation (sûre, détectée en CI), donc
    // pas de risque à laisser tel quel en attendant. Le filet de sécurité réel contre une adresse
    // absente/fausse est documenté plus haut (échec propre au chargement, pas un crash).
    aInfo->runtime = RED4EXT_V1_RUNTIME_VERSION_INDEPENDENT;
    aInfo->sdk = RED4EXT_V1_SDK_VERSION_CURRENT;
}

// Main — appelé au chargement et au déchargement du plugin par le loader.
RED4EXT_C_EXPORT bool RED4EXT_CALL Main(RED4ext::v1::PluginHandle aHandle, RED4ext::v1::EMainReason aReason,
                                        const RED4ext::v1::Sdk* aSdk)
{
    switch (aReason)
    {
    case RED4ext::v1::EMainReason::Load:
        aSdk->logger->Info(aHandle,
            "Tessera chargé — plugin client minimal v0.1.0. La chaîne de livraison fonctionne.");

        // Désossage natif phase 1 (log-only) — voir le commentaire complet plus haut.
        TesseraDesossageNative::g_sdk = aSdk;
        TesseraDesossageNative::g_handle = aHandle;
        {
            RED4ext::UniversalRelocFunc<TesseraDesossageNative::ExecuteNode_t> target(
                TesseraDesossageNative::kExecuteNodeHash);
            // UniversalRelocFunc n'expose que `operator T()` (SDK 1.0.0, Relocation.hpp:126) — pas
            // de GetAddr() (réservé à UniversalRelocPtr, ligne 153). reinterpret_cast<void*>(objet)
            // n'invoque PAS l'opérateur de conversion défini par l'utilisateur → C2440. Il faut donc
            // d'abord matérialiser le pointeur de fonction résolu via static_cast (qui, lui, appelle
            // operator T()), puis le caster en void* pour Attach (Hooking.hpp:47 :
            // Attach(PluginHandle, void* aTarget, void* aDetour, void** aOriginal)).
            auto targetFn = static_cast<TesseraDesossageNative::ExecuteNode_t>(target);
            bool attached = aSdk->hooking->Attach(aHandle, reinterpret_cast<void*>(targetFn),
                reinterpret_cast<void*>(&TesseraDesossageNative::Detour),
                reinterpret_cast<void**>(&TesseraDesossageNative::g_original));
            if (attached)
            {
                aSdk->logger->Info(aHandle,
                    "[Tessera/DesossageNative] hook ExecuteNode attaché (phase 1, log-only).");
            }
            else
            {
                aSdk->logger->Info(aHandle,
                    "[Tessera/DesossageNative] ERREUR : échec d'attache du hook ExecuteNode.");
            }
        }
        break;

    case RED4ext::v1::EMainReason::Unload:
        aSdk->logger->Info(aHandle, "Tessera déchargé.");
        break;
    }

    return true;
}

// Supports — version d'API que ce plugin attend du loader (API v1).
RED4EXT_C_EXPORT uint32_t RED4EXT_CALL Supports()
{
    return RED4EXT_API_VERSION_1;
}
