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

## Barre système

- L'icône indique l'état : micro Discord **actif**, **muté** ou application **en pause**.
- Le menu affiche le **profil en cours d'utilisation**, avec un raccourci pour le désactiver.
- Le menu permet aussi de mettre l'application en pause, d'ouvrir la fenêtre des réglages et de quitter.

## Langues

- **Anglais** et **français**.
- Langue du système par défaut, modifiable dans les réglages.
- Traductions dans des fichiers séparés, pour pouvoir ajouter d'autres langues plus tard.

## Préréglages (50 jeux avec chat vocal intégré)

Seuls les jeux qui ont **leur propre chat vocal** sont retenus.

Légende : ✅ = touche par défaut confirmée · ⚠️ = touche à vérifier

| # | Jeu | Touche par défaut |
|---|---|---|
| 1 | Counter-Strike 2 | K ✅ |
| 2 | PUBG: Battlegrounds | T ⚠️ |
| 3 | Fortnite | T ⚠️ |
| 4 | Apex Legends | T ⚠️ |
| 5 | Valorant | V (équipe) ✅ / U (groupe) ⚠️ |
| 6 | Call of Duty: Warzone / Black Ops 7 | Z ⚠️ |
| 7 | Rainbow Six Siege X | ⚠️ |
| 8 | Overwatch 2 | ⚠️ |
| 9 | Marvel Rivals | ⚠️ |
| 10 | Battlefield 6 | ⚠️ |
| 11 | Deadlock | K ⚠️ |
| 12 | The Finals | ⚠️ |
| 13 | Delta Force | ⚠️ |
| 14 | Escape from Tarkov | K ⚠️ |
| 15 | Arc Raiders | ⚠️ |
| 16 | Hunt: Showdown 1896 | ⚠️ |
| 17 | Team Fortress 2 | V ✅ |
| 18 | Destiny 2 | ⚠️ |
| 19 | Helldivers 2 | ⚠️ |
| 20 | Naraka: Bladepoint | ⚠️ |
| 21 | Squad | ⚠️ (plusieurs canaux) |
| 22 | Hell Let Loose | ⚠️ (plusieurs canaux) |
| 23 | Insurgency: Sandstorm | ⚠️ |
| 24 | Ready or Not | ⚠️ |
| 25 | Left 4 Dead 2 | ⚠️ |
| 26 | Warframe | ⚠️ |
| 27 | War Thunder | ⚠️ |
| 28 | World of Tanks | ⚠️ |
| 29 | Marathon | ⚠️ |
| 30 | Payday 2 | ⚠️ |
| 31 | Arena Breakout: Infinite | ⚠️ |
| 32 | Dota 2 | ⚠️ |
| 33 | World of Warcraft | ⚠️ |
| 34 | Sea of Thieves | Alt gauche ✅ |
| 35 | Rust | V ✅ |
| 36 | DayZ | Verr. Maj ✅ |
| 37 | Arma 3 | Verr. Maj ✅ |
| 38 | Arma Reforger | ⚠️ |
| 39 | GTA Online | N ✅ |
| 40 | FiveM (GTA RP) | N ⚠️ (dépend du serveur) |
| 41 | Garry's Mod | V ✅ |
| 42 | Roblox | ⚠️ (vocal réservé aux comptes vérifiés) |
| 43 | Dark and Darker | ⚠️ |
| 44 | Lockdown Protocol | ⚠️ |
| 45 | Lethal Company | T ✅ |
| 46 | Phasmophobia | V ✅ |
| 47 | Content Warning | ⚠️ |
| 48 | R.E.P.O. | ⚠️ |
| 49 | PEAK | ⚠️ |
| 50 | Deep Rock Galactic | ⚠️ |

## Points restant à traiter

- Vérifier les touches marquées ⚠️ et les noms d'exécutables de chaque jeu (Windows et Linux/Proton).
- Publier `discord-voice-rpc` sur GitHub, puis y faire pointer les deux projets (dépendance par chemin local en attendant).
- Plus tard : migrer `goxlr-discord-sync` vers `discord-voice-rpc` (entropie DPAPI `b"goxlr-discord-sync"` pour relire les secrets existants).
- Décider si l'option PipeWire/PulseAudio (Linux) est retenue.
