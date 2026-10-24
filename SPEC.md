# Discord Auto-Mute — Cahier des charges

## Objectif

Quand on joue avec des amis présents à la fois dans un salon vocal Discord et dans le chat vocal d'un jeu, ils nous entendent en double. L'application mute automatiquement le micro Discord quand on utilise la touche « parler » du jeu.

## Plateformes et technologie

- Langage : **Rust**
- Systèmes : **Windows** et **Linux**
  - Linux X11 : support complet
  - Linux Wayland : KDE (via D-Bus) puis GNOME (via une extension) pour détecter la fenêtre au premier plan ; écoute du clavier via evdev (utilisateur dans le groupe `input`) ou via le portail « raccourcis globaux »
- L'application tourne en arrière-plan avec une icône dans la **barre système** (system tray).

## Structure du projet

Même organisation que `goxlr-discord-sync` :

- `crates/core` (`dam-core`) : logique pure, testée sans interface ni système.
  - `profile` : profils, préréglages (`presets.json`), jokers `*` dans les noms d'exécutables.
  - `engine` : règles de mute. Entrées : touches, processus au premier plan, état du micro Discord, pause, profils. Sortie : « muter » ou « réactiver ».
  - `config` : `settings.json` (langue, méthode, raccourci, client ID, profils) et opérations sur les profils.
  - `i18n` : catalogues `i18n/<code>.json`, langue du système par défaut.
  - `keys` : codes de touche physiques (noms de `KeyboardEvent.code`, plus `MouseMiddle`, `Mouse4`, `Mouse5`).
- `src-tauri` : application Tauri 2.
  - `host` : thread du moteur ; reçoit touches, premier plan, réglages et état Discord sous forme d'`Event`.
  - `platform` : Windows (hooks bas niveau clavier/souris, `GetForegroundWindow`, `SendInput`) et Linux (evdev, X11 `_NET_ACTIVE_WINDOW` + `/proc/<pid>/cmdline` pour Proton, XTest).
  - `discord` : session RPC (connexion, autorisation, reconnexion, suivi de l'état du micro).
  - `tray` : icône (prêt / muté / inactif / pause / problème) et menu.
  - `settings` + `ui/` : fenêtre de réglages (profils enregistrés immédiatement, onglets Discord et Général avec Enregistrer).
- `tools/make_icons.py` : génère les icônes (Pillow).
- Test `src-tauri/tests/i18n.rs` : chaque texte de l'interface a sa traduction française, sans entrée inutilisée.

## État des tests

- Logique (moteur, réglages, préréglages, touches, traductions) : tests automatiques.
- Module Windows : compile pour Windows, **jamais exécuté** sous Windows.
- Linux : fenêtre de réglages vérifiée sous WSLg ; écoute du clavier, détection du jeu et envoi du raccourci **jamais testés** sur un vrai bureau Linux.
- Mode « application » : jamais testé avec un vrai client Discord.

## Méthodes pour muter Discord (au choix de l'utilisateur)

1. **Raccourci** : l'application envoie un raccourci configuré dans Discord. Deux actions Discord possibles :
   - « Activer/désactiver le micro » : un appui pour muter, un autre pour réactiver ;
   - « Appuyer pour muter » : le raccourci reste enfoncé tant que Discord doit rester muté.
   Par défaut, le raccourci est F13 : aucun jeu n'utilise cette touche, donc l'envoyer ne déclenche rien dans le jeu. Avec cette méthode, l'application ne connaît pas l'état réel du micro Discord.
2. **Application** : l'application passe par l'API locale de Discord (RPC, `SET_VOICE_SETTINGS` avec la permission `rpc.voice.write`). L'utilisateur fournit le client ID et le secret de sa propre application développeur Discord. Le code vient de la bibliothèque commune `../discord-voice-rpc/`, extraite de `goxlr-discord-sync` :
   - `ipc` : connexion IPC (pipe nommé `discord-ipc-N` sous Windows, socket Unix sous Linux), `AUTHORIZE`, `AUTHENTICATE`, `GET/SET_VOICE_SETTINGS`, abonnement aux événements ;
   - `auth` : autorisation OAuth dans le navigateur (pages traduisibles via `CallbackPages`), échange du code, rafraîchissement du jeton, vérification du client ID et du secret (`CredentialCheck`, à traduire par l'application) ;
   - `secrets` : stockage du secret client et des jetons (DPAPI sous Windows, fichier 0600 sous Linux), avec une entropie propre à chaque application.
3. *(Option à étudier, Linux seulement)* : couper le flux micro de Discord via PipeWire ou PulseAudio.

## Profils

- **Un profil par jeu**, identifié par l'exécutable du jeu.
- Un profil vient soit d'un **préréglage**, soit d'une **création custom** (profil vide).
- Champs d'un profil :
  - **Nom** : obligatoire et modifiable ; prérempli avec le nom du jeu pour un préréglage
  - **Activé / désactivé** : un profil désactivé est ignoré, mais ses réglages sont conservés
  - **Exécutable(s)** du jeu
  - **Une ou plusieurs touches** pour parler : clavier, y compris une touche de modification seule (par ex. Alt gauche, distinguée d'Alt droite / AltGr), et boutons de souris (Souris 4 et 5)
  - **Mode** : *maintenu* (Discord est muté tant que la touche est enfoncée) ou *bascule* (un appui mute, l'appui suivant réactive)
- Les préréglages sont des **points de départ** : tous leurs champs sont modifiables.
- Bouton **« Rétablir les valeurs par défaut »** pour un profil issu d'un préréglage.
- Les mises à jour de la liste des préréglages **n'écrasent jamais** les profils modifiés par l'utilisateur ; seuls les profils non modifiés sont mis à jour.
- Plusieurs profils peuvent exister pour un même jeu, mais **un seul peut être activé à la fois** : en activer un désactive automatiquement les autres.

## Comportement

- La touche est **seulement écoutée, jamais bloquée** : le jeu la reçoit normalement.
- Un profil ne s'applique que quand **son jeu est la fenêtre au premier plan**. Sinon, sa touche est ignorée.
- Quand le jeu passe en arrière-plan alors que Discord est muté par l'application :
  - en mode **maintenu** : Discord est réactivé immédiatement ;
  - en mode **bascule** : Discord est réactivé (retour à l'état d'avant). En revenant dans le jeu, l'utilisateur doit réappuyer sur la touche.
- Règle générale : l'application ne laisse **jamais** l'utilisateur muté sur Discord sans qu'il le sache.
- L'application ne réactive que ce qu'elle a muté. Avec la méthode Application, elle reconnaît les confirmations de ses propres commandes (un retard n'est pas pris pour une action de l'utilisateur) et rejoue une réactivation qui a échoué dès que Discord répond.
- Si l'application s'arrête alors que Discord est muté (crash, fin de tâche), `state.json` le garde en mémoire et Discord est réactivé au démarrage suivant.
- Méthode Raccourci : « Appuyer pour rendre muet » par défaut (suit la touche, ne peut pas s'inverser) ; « Activer/désactiver le micro » reste possible. Un bandeau rappelle de créer le raccourci dans Discord tant que l'utilisateur ne l'a pas confirmé, et un bouton « Tester » mute Discord 3 s.

## Barre système

- L'icône indique l'état : micro Discord **actif**, **muté** ou application **en pause**.
- Le menu affiche le **profil en cours d'utilisation**, avec un raccourci pour le désactiver.
- Le menu permet aussi de mettre l'application en pause, d'ouvrir la fenêtre des réglages et de quitter.

## Langues

- **Anglais** et **français**.
- Langue du système par défaut, modifiable dans les réglages.
- Traductions dans des fichiers séparés, pour pouvoir ajouter d'autres langues plus tard.

## Préréglages (50 jeux avec chat vocal intégré)

Seuls les jeux qui ont **leur propre chat vocal** sont retenus. La source de vérité est `crates/core/presets.json` ; touches, exécutables et AppID Steam viennent d'une recherche (wikis, documentation officielle, fichiers de configuration, Steam appinfo) faite le 24/10/2026.

Légende : ✅ = touche confirmée par une source fiable · ⚠️ = source secondaire ou incertaine, à vérifier · — = pas de touche connue (micro ouvert ou touche non assignée)

| # | Jeu | Touche(s) par défaut | Exécutable(s) |
|---|---|---|---|
| 1 | Counter-Strike 2 | K ✅ | `cs2.exe`, `cs2` |
| 2 | PUBG: Battlegrounds | T ⚠️ | `TslGame.exe` |
| 3 | Fortnite | — ⚠️ (micro ouvert par défaut) | `FortniteClient-Win64-Shipping.exe` |
| 4 | Apex Legends | T ✅ | `r5apex.exe`, `r5apex_dx12.exe` |
| 5 | Valorant | V, U ✅ | `VALORANT-Win64-Shipping.exe` |
| 6 | Call of Duty: Warzone / Black Ops 7 | Z ⚠️ | `cod.exe` |
| 7 | Rainbow Six Siege X | U ⚠️ | `RainbowSix*.exe` |
| 8 | Overwatch 2 | ² / ` ⚠️ | `Overwatch.exe` |
| 9 | Marvel Rivals | — ⚠️ (micro ouvert par défaut) | `Marvel-Win64-Shipping.exe` |
| 10 | Battlefield 6 | Verr. Maj ⚠️ | `bf6.exe` |
| 11 | Deadlock | T ⚠️ | `deadlock.exe` |
| 12 | The Finals | Z ⚠️ | `Discovery*.exe` |
| 13 | Delta Force | T ⚠️ | `DeltaForceClient-Win64-Shipping.exe` |
| 14 | Escape from Tarkov | K ✅ | `EscapeFromTarkov.exe` |
| 15 | Arc Raiders | Z, B ✅ | `PioneerGame*.exe` |
| 16 | Hunt: Showdown 1896 | Verr. Maj ⚠️ | `HuntGame.exe` |
| 17 | Team Fortress 2 | V ✅ | `tf_win64.exe`, `tf.exe`, `tf_linux64` |
| 18 | Destiny 2 | T ⚠️ (micro ouvert par défaut) | `destiny2.exe` |
| 19 | Helldivers 2 | Verr. Maj ⚠️ | `helldivers2.exe` |
| 20 | Naraka: Bladepoint | — ⚠️ | `NarakaBladepoint.exe` |
| 21 | Squad | V, B, G ⚠️ | `SquadGame.exe` |
| 22 | Hell Let Loose | C, V, X ✅ | `HLL*-Win64-Shipping.exe` |
| 23 | Insurgency: Sandstorm | K ⚠️ | `InsurgencyClient-Win64-Shipping.exe` |
| 24 | Ready or Not | — ⚠️ | `ReadyOrNot*-Win64-Shipping.exe` |
| 25 | Left 4 Dead 2 | C ✅ | `left4dead2.exe` |
| 26 | Warframe | C ✅ | `Warframe.x64.exe` |
| 27 | War Thunder | Ctrl droit ⚠️ | `aces.exe`, `aces` |
| 28 | World of Tanks | Q ✅ | `WorldOfTanks.exe` |
| 29 | Marathon | Y, T ✅ | `Marathon.exe` |
| 30 | Payday 2 | ² / ` ⚠️ | `PAYDAY2.exe`, `payday2_win32_release.exe` |
| 31 | Arena Breakout: Infinite | Y ⚠️ | `UAGame.exe` |
| 32 | Dota 2 | G ✅ | `dota2.exe`, `dota2` |
| 33 | World of Warcraft | ² / ` ✅ | `Wow.exe`, `WowClassic.exe` |
| 34 | Sea of Thieves | Alt gauche ✅ (micro ouvert par défaut) | `SoTGame.exe` |
| 35 | Rust | V ✅ | `RustClient.exe` |
| 36 | DayZ | Verr. Maj ✅ | `DayZ_x64.exe` |
| 37 | Arma 3 | Verr. Maj ✅ | `arma3_x64.exe`, `arma3.exe` |
| 38 | Arma Reforger | T, Verr. Maj ⚠️ | `ArmaReforger*.exe` |
| 39 | GTA Online | N ✅ | `GTA5.exe`, `GTA5_Enhanced.exe` |
| 40 | FiveM (GTA RP) | N, Alt gauche ⚠️ | `FiveM_*GTAProcess.exe` |
| 41 | Garry's Mod | X, Souris 5 ✅ | `gmod.exe`, `gmod_win64.exe` |
| 42 | Roblox | — ⚠️ (micro ouvert par défaut) | `RobloxPlayerBeta.exe` |
| 43 | Dark and Darker | B ⚠️ | `DungeonCrawler.exe` |
| 44 | Lockdown Protocol | — ⚠️ (micro ouvert par défaut) | `LockdownProtocol-Win64-Shipping.exe` |
| 45 | Lethal Company | T ✅ | `Lethal Company.exe` |
| 46 | Phasmophobia | V, B ✅ | `Phasmophobia.exe` |
| 47 | Content Warning | V ✅ (micro ouvert par défaut) | `Content Warning.exe` |
| 48 | R.E.P.O. | V ✅ | `REPO.exe` |
| 49 | PEAK | V ✅ (micro ouvert par défaut) | `PEAK.exe` |
| 50 | Deep Rock Galactic | Z ✅ | `FSD-Win*-Shipping.exe` |

## Points restant à traiter

- Vérifier en jeu les touches marquées ⚠️ (message Discord aux joueurs).
- Détection des jeux installés : Steam et Epic d'abord, puis Xbox / Microsoft Store.
- Plus tard : migrer `goxlr-discord-sync` vers `discord-voice-rpc` (entropie DPAPI `b"goxlr-discord-sync"` pour relire les secrets existants).
- Décider si l'option PipeWire/PulseAudio (Linux) est retenue.
