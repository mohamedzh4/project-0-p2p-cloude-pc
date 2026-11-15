# Cahier des Charges — Distributed Compute Lab Platform

## 1. Contexte

Traditional cloud computing routes every workload through centralized datacenter servers. This project instead lets individuals with underused high-performance PCs share a slice of that idle capacity with other users — while they keep using the machine themselves — over a peer network, accessed as an on-demand virtual desktop through a browser.

## 2. Objectifs du Projet

- Let a user with a low-spec PC access a high-performance virtual desktop, matched automatically to the best available peer host
- Let a host monetize idle CPU/RAM, and optionally a GPU they can spare, with near-zero manual effort while they keep using the rest of the machine
- Guarantee that neither side compromises the other: the host never sees the user's data, and the host retains nothing once a session ends
- Deliver a session that feels indistinguishable from working on the physical machine, for both office-grade and GPU-heavy workloads

## 3. Parties Prenantes

| Rôle | Description |
|---|---|
| **Utilisateur** | Rents a virtual desktop session; brings an account, optional persistent files, and a task ("what do you want to do today") |
| **Host / Provider** | Keeps using their PC. Shares a declared CPU/RAM slice, and optionally dedicates one GPU for the session (that GPU is then unavailable to them until the session ends). Runs the Host Agent |
| **Plateforme (opérateur)** | Runs the Matchmaker, billing, base-image pipeline, and network infrastructure — the only centralized piece |

## 4. Périmètre

### Inclus dans le MVP
- Standard Lab — QEMU VM with the accelerator chosen by host OS: QEMU+KVM + Kasm on Linux hosts, QEMU+WHPX + Kasm on Windows hosts
- Pro/Gamer Lab — exclusive GPU assignment for the session: QEMU/KVM + VFIO on Linux, Hyper-V DDA/GPU-P (or QEMU+WHPX passthrough) on Windows, plus Sunshine/Moonlight
- Web app for users, tray app for hosts
- Matchmaker: auth, intent capture, matching, session lifecycle, billing
- Persistent storage: platform storage on Cloudflare R2 (S3-compatible), encrypted with rclone before upload; the user can instead save to their own Google Drive or Dropbox, and can always download their files to their own computer (FR20). Storj is added later as the decentralized backend. Sia/renterd is dropped so the platform needs no crypto payments (decision 2026-11-15, replaces the 2026-09-05 renterd choice)
- Secure mesh networking (Tailscale/Headscale)
- Direct peer-to-peer desktop streaming (WebRTC) between the user's browser and the host's guest VM; the platform relays the stream only as a fallback (decision 2026-11-15, see FR17)

### Exclu du MVP (phases futures)
- Confidential computing tier (SEV-SNP/TDX-aware routing)
- Multi-region redundancy for the control plane
- Native mobile clients
- Enterprise SSO / team accounts
- Regional platform relays and volunteer relay nodes (FR19) — growth phase

## 5. Besoins Fonctionnels

### 5.1 Côté Utilisateur
- FR1 — Create an account and log in (resolved internally to `User ID X`)
- FR2 — Be asked "What do you want to do in your lab today?" and have the answer routed to a lab tier
- FR3 — Be automatically matched to the most suitable available host
- FR4 — Access previously stored files inside the session, transparently and encrypted
- FR5 — Work in a live streamed desktop session with no perceptible lag for the Standard tier
- FR6 — Log out with a guaranteed final save; see a "saved as of HH:MM" indicator during the session
- FR7 — View session history and billing/credits
- FR20 — Choose where session files are kept, and get them back. The user picks one of:
  - **Platform storage** (default): Cloudflare R2 now, Storj later, encrypted before upload, with a free quota (e.g. 20 GB) and extra space paid in credits.
  - **My Google Drive** or **My Dropbox**: the user connects their own account once; the session mounts a dedicated app folder in that account, so the platform stores nothing and the session can reach only that folder. Encryption is the user's choice (encrypted files cannot be opened directly in Drive/Dropbox).
  - Whatever they pick, the user can **download their files to their own computer** at any time from a "My files" page in the web app (single files or everything as one zip, also offered at logout) and **upload** files from their computer before or during a session.

### 5.2 Côté Host/Provider
- FR8 — Register and declare exactly how much RAM and CPU to share (a concurrent slice; the host keeps the rest), and optionally which GPU to dedicate. GPU sharing is exclusive for the session on consumer hardware — the host loses that GPU until the session ends. An availability schedule is specified later; the declared GPU identity must already be stored so that schedule does not require a breaking config change.
- FR9 — Have the Host Agent auto-join the network mesh and download the correct signed base image without manual steps
- FR10 — See a simple status view (idle / in-session / earnings) with nothing to operate mid-session
- FR11 — Pause or stop sharing at any time

### 5.3 Côté Plateforme (Matchmaker)
- FR12 — Track real-time host inventory: resources, load, reliability score
- FR13 — Match on tier fit, latency, and reliability, in that priority order for the Pro tier
- FR14 — Provision, monitor (heartbeat), and tear down sessions automatically
- FR15 — Re-match a user to a new host within the bounded data-loss window if a host disconnects
- FR16 — Meter usage and settle billing/payouts
- FR17 — Stream the desktop **directly** between the user's browser and the guest VM on the host (WebRTC, peer-to-peer, no install). The platform carries only control traffic — login, matching, connection set-up (signaling), heartbeats — and never the video stream when a direct path exists. Fallbacks, chosen automatically per session: a platform TURN relay when no direct path can be found (CGNAT, strict firewalls), then the existing HTTPS gateway. The session token is checked during signaling so a user can only connect to their own session. Whether a direct path is allowed at all is decided by FR18.
- FR18 — Privacy choice. Each user and each host chooses, in their settings and per session, between **"Hide my IP"** — the stream goes only through platform-operated relays, so neither side (nor any third party) learns the other's public IP — and **"Fastest connection"** — direct peer-to-peer is allowed, and in the growth phase a volunteer relay (FR19) may be used. A session goes direct only when **both** sides chose "Fastest connection"; otherwise it uses a platform relay. Default: "Hide my IP". The choice is explained in plain words ("the other side will see your internet address") and can be changed at any time.
- FR19 — Volunteer relay nodes (growth phase). A host with a public IP and enough upload bandwidth can opt in to relay other sessions' encrypted streams and earn credits per GB relayed. The Matchmaker picks a relay close to both sides, scores relays on reliability, and bans bad ones. Volunteer relays carry only sessions where both sides chose "Fastest connection" (FR18), because a relay sees both public IPs. Relays forward end-to-end encrypted traffic and cannot read it.

## 6. Besoins Non-Fonctionnels

| Catégorie | Exigence |
|---|---|
| Performance | The desktop stream takes the direct host ↔ user path whenever one exists (FR17); relaying through the platform is a fallback, not the default. Standard Lab sessions must feel locally responsive for office/browsing/coding workloads; Pro/Gamer sessions need latency low enough for real-time video/gaming (target sub-50ms round trip where possible) |
| Sécurité | Zero data retention on host after logout; encryption at rest and in transit; signed, verified base images; per-session network isolation |
| Fiabilité | Bounded data-loss window on host disconnect (event-triggered sync, not just the 5–10 min timer) |
| Scalabilité | Adding hosts or users must not require redesigning the four-plane architecture. Platform bandwidth must grow with *relayed* sessions only, not with every session (FR17) |
| Confidentialité | In direct mode the user's browser and the host PC learn each other's public IP. This must be disclosed in the host agreement and the user terms. FR18 lets each side choose "Hide my IP" (platform relay only, the default) or "Fastest connection" (direct, or a volunteer relay in the growth phase) |
| Disponibilité | The Matchmaker (control plane) is the one centralized piece and needs a real uptime target, even though compute is decentralized |
| Utilisabilité | Zero install for the Standard Lab (browser only); minimal friction accepted for the Pro/Gamer Lab (native client tolerable) |

## 7. Architecture Technique (Résumé)

| Plan | Rôle | Outils |
|---|---|---|
| Contrôle | Auth, matching, billing | Matchmaker (custom) |
| Hébergement plateforme | Serveur central : Matchmaker + Postgres, Headscale, passerelle HTTPS (Caddy), relais TURN, point d'arrivée des tunnels SSH des hosts | **Oracle Cloud Infrastructure, offre Always Free** pour la phase de test (instance ARM Ampere A1, Ubuntu 24.04, région proche du Maroc : Madrid ou Marseille, IP publique réservée). Remplace Azure (essai gratuit désactivé le 2026-11-15). Ensuite n'importe quel VPS Ubuntu (Hetzner / OVH, ou Fluence en option web3) : `scripts/vps-deploy.sh` ne dépend d'aucun fournisseur |
| Calcul | Exécute les sessions | Standard: QEMU+KVM (Linux host) or QEMU+WHPX (Windows host). Pro: QEMU+KVM+VFIO (Linux) or Hyper-V GPU assignment (Windows). GPU assignment is exclusive for the session. |
| Réseau | Connecte user↔VM↔storage | Tailscale / Headscale (host, guest, storage, control); public HTTPS gateway as last-resort path for the stream |
| Streaming | Desktop video + mouse/keyboard | Direct WebRTC browser ↔ guest (Selkies-GStreamer in the guest, Standard and Pro); signaling through the Matchmaker; coturn TURN relay on the platform for failed direct paths and for "Hide my IP" sessions (FR18); regional platform relays and volunteer relay nodes (FR19) in the growth phase. KasmVNC behind the gateway stays as the fallback |
| Stockage | Persistance des données | rclone in the guest VM. Platform storage: Cloudflare R2 (S3-compatible) now, Storj (decentralized, paid in normal money) later. Optional user-owned storage: Google Drive, Dropbox. Download/upload to the user's computer through the web app |

*Voir le document "Distributed Compute Lab — Complete Architecture Specification" pour le détail technique complet, incluant la table des vulnérabilités identifiées et leurs correctifs (12 items).*

## 8. Contraintes Techniques et Légales

- Standard Lab runs QEMU on both host OSes: KVM on Linux, WHPX on Windows. The guest uses QEMU user-mode networking, so the volunteer's PC needs no root-owned network device (decision 2026-11-15; Firecracker was dropped because it needs a separate kernel and a TAP device)
- Direct streaming must cross two NATs: QEMU user-mode networking inside the host, then the host's home router. The Host Agent forwards one UDP port into the guest for WebRTC; whether a direct path forms is only known after testing on real, separate networks. Mobile networks and CGNAT ISPs usually need the TURN relay. Renter files (rclone ↔ storage) and guest internet traffic (exit node) still pass through platform infrastructure; FR17 covers the desktop stream only
- A user's Google Drive / Dropbox access token reaches the guest VM, which runs on a volunteer's PC. Only app-folder access is requested (Google Drive `drive.file`, Dropbox "App folder"), never the whole account, so a dishonest host could at worst reach that one folder. Google and Dropbox app registration (and Google's consent-screen review) is required before public use
- CPU/RAM sharing is concurrent with the host's own use. GPU sharing on consumer hardware is exclusive for the session (passthrough / whole-device assignment). Realistic options: a second GPU, or a later schedule when the host is away. SR-IOV/vGPU is out of scope
- Pro/Gamer hosts on Linux require an IOMMU-capable CPU/motherboard (VT-d / AMD-Vi). On Windows they need a GPU the hypervisor can assign, plus a remaining display path (iGPU or a second card) so the host can keep using the PC
- Kernel-level anti-cheat (EAC, BattlEye, Vanguard) blocks some titles by vendor policy — not something the architecture can route around
- Consumer GPU driver EULAs (NVIDIA/AMD) may restrict commercial "cloud gaming"-style use — requires legal review before commercial launch
- Guest network egress must not exit through the host's raw ISP connection — routed through a platform-controlled exit node instead, to avoid exposing hosts to legal liability for a user's traffic
- Terms of Service, acceptable-use policy, and host indemnification require legal review before public launch
- Test server on Oracle Cloud Always Free: the free ARM allowance is limited (reported as 2 OCPU / 12 GB since August 2026 — check Oracle's current free-tier page), Oracle may reclaim an instance that stays idle (reported: CPU, network and memory all under ~20% over 7 days), a card is required for identity verification only, and ports must be opened twice — in the OCI security list *and* in Ubuntu's own firewall. The home region cannot be changed after sign-up. Before real users, move to a paid VPS or a paid Oracle account; keep database and key backups so a move or a reclaim loses nothing

## 9. Stratégie de Développement — Par Où Commencer

The Matchmaker (backend) is conventional, low-risk engineering. The compute pipeline — QEMU+KVM+Kasm feeling native on Linux, QEMU+WHPX+Kasm feeling native on Windows, VFIO/Hyper-V GPU assignment + Sunshine hitting playable latency — is where the real technical risk lives. Development should follow risk first, not layer first:

1. Manually validate one Standard session on Linux (QEMU+KVM+Kasm) and one on Windows (QEMU+WHPX+Kasm), plus one Pro session (QEMU+VFIO or Hyper-V GPU assignment + Sunshine) — no server, no UI — to confirm the core technical bet holds
2. Build a minimal Matchmaker: one hardcoded host, one user, provisioning triggered by code instead of by hand
3. Build the client web app around that working loop
4. Generalize: real matching across many hosts, billing, automated host onboarding

## 10. Livrables

- Application web utilisateur
- Application hôte (tray app)
- Service backend Matchmaker + APIs
- Pipeline de build et signature des images de base
- Documentation (guide utilisateur, guide host, documentation API)

## 11. Jalons / Phases

| Phase | Contenu |
|---|---|
| Phase 0 | Preuve de concept manuelle du pipeline de calcul (un seul host, une seule session) |
| Phase 1 | MVP Standard Lab, région unique, hosts approuvés manuellement. Serveur de test : Oracle Cloud Always Free |
| Phase 2 | Matchmaker complet (matching + billing), onboarding host automatisé. Stockage : Cloudflare R2 avec quota par utilisateur, page « Mes fichiers » (téléchargement / envoi vers l'ordinateur de l'utilisateur), option Google Drive / Dropbox (FR20) |
| Phase 3 | Ajout du Pro/Gamer Lab, matching sensible à la latence. Streaming direct pair-à-pair WebRTC (FR17) pour Standard et Pro, relais TURN et passerelle HTTPS en secours, choix de confidentialité « Masquer mon IP » / « Connexion la plus rapide » (FR18) |
| Phase 4 | Durcissement sécurité (signature d'image, exit nodes, scoring de fiabilité), tier confidential computing si demandé par le marché |
| Phase 5 | Croissance : stockage Storj (décentralisé) en plus de R2, choix de la région de stockage par utilisateur; relais de la plateforme dans plusieurs régions proches des utilisateurs, puis nœuds relais volontaires rémunérés en crédits (FR19), utilisés seulement pour les sessions « Connexion la plus rapide » |

## 12. Critères d'Acceptation

- Un utilisateur obtient une session Standard Lab utilisable en moins de quelques secondes après sélection
- Une déconnexion host en cours de session entraîne une perte de données bornée (quelques minutes maximum) et un re-matching automatique
- Aucune trace des données utilisateur ne subsiste sur le disque physique du host après une session
- Le host n'a aucune action à effectuer pendant une session active, hormis rester allumé et connecté — and they keep using the rest of the machine, except a GPU they explicitly dedicated

## 13. Risques Principaux

| Risque | Impact | Mitigation |
|---|---|---|
| Le trafic sortant de la VM expose l'IP du host | Exposition légale d'un hôte innocent | Routage via exit node contrôlé par la plateforme |
| Absence de confidential computing par défaut | Le host pourrait théoriquement inspecter la mémoire d'une VM | Router les sessions sensibles vers des hosts SEV-SNP/TDX quand disponibles |
| Incompatibilité anti-cheat sur certains jeux | Une partie du catalogue gaming ne fonctionnera pas | Publier une liste de compatibilité, ne pas tenter de contourner les politiques éditeurs |
| Licences des pilotes GPU grand public | Restriction possible d'usage commercial | Revue juridique avant lancement commercial |
| La connexion directe échoue (double NAT via QEMU, CGNAT, pare-feu strict) | Le flux passe par le relais : plus de latence et de bande passante côté plateforme | Relais TURN (coturn) puis passerelle HTTPS en secours automatique; mesurer la part de sessions relayées; transfert d'un port UDP vers la VM |
| En mode direct, l'utilisateur et le host voient l'IP publique l'un de l'autre | Exposition de l'adresse du host volontaire (et de l'utilisateur) | Choix FR18 : « Masquer mon IP » par défaut (relais de la plateforme seulement); direct uniquement si les deux côtés l'acceptent; l'indiquer dans l'accord host et les CGU |
| Un nœud relais volontaire (FR19) voit les IP des deux côtés, peut ralentir ou couper le flux, ou s'éteindre | Fuite d'IP vers un tiers, session dégradée | Relais volontaires réservés aux sessions « Connexion la plus rapide »; trafic chiffré de bout en bout; score de fiabilité et bannissement; reconnexion automatique par un autre chemin |
| Le jeton Google Drive / Dropbox de l'utilisateur passe par la VM chez un host | Un host malhonnête pourrait lire ce jeton | Accès limité au dossier de l'application (pas tout le compte), jeton révocable par l'utilisateur, durée courte quand c'est possible |
| Stockage plateforme chez une seule entreprise (Cloudflare R2) | Dépendance à un fournisseur | Protocole S3 standard : changement de fournisseur par configuration; Storj ajouté en phase de croissance; fichiers chiffrés avant envoi |
| Le serveur de test gratuit (Oracle Always Free) est récupéré pour inactivité ou ses limites changent | Plateforme injoignable, comme l'arrêt d'Azure le 2026-11-15 | Sauvegardes de la base et des clés Headscale; script de déploiement indépendant du fournisseur; passage à un VPS payant avant les vrais utilisateurs |
| GPU passthrough exclusif sur matériel grand public | Le host perd ce GPU (affichage, ses propres apps) pendant toute la session | UI honnête; 2e GPU ou iGPU, ou un créneau plus tard. Ne pas présenter le GPU comme un slice concurrent CPU/RAM. SR-IOV/vGPU hors périmètre |

## 14. Glossaire

- **Matchmaker** — Service central qui authentifie les utilisateurs, fait correspondre demande et host, et pilote le cycle de vie des sessions
- **Host Agent** — Application légère installée chez le host, qui rejoint le réseau maillé et déclenche le provisioning des VMs
- **Base Image** — Image système standby, signée, prête à démarrer instantanément sur un host
- **KVM** — Hyperviseur intégré au noyau Linux. Backend QEMU pour le Standard Lab (et, avec VFIO, pour le Pro Lab) sur un host Linux.
- **WHPX** — Windows Hypervisor Platform. Backend QEMU pour le Standard Lab sur un host Windows, sans installer Linux sur la machine du volontaire.
- **VFIO** — Mécanisme du noyau Linux permettant le passthrough direct d'un GPU physique vers une VM. Exclusive pour la durée de la session.
- **WebRTC** — Technologie standard des navigateurs pour la vidéo temps réel pair-à-pair, sans installation, avec traversée NAT
- **Signaling** — Échange initial (quelques Ko) des informations de connexion entre navigateur et VM, via le Matchmaker, avant que le flux direct démarre
- **TURN (coturn)** — Serveur relais utilisé seulement quand aucune connexion directe n'est possible
- **Nœud relais volontaire** — PC d'un host (IP publique, bon débit montant) qui relaie des flux chiffrés d'autres sessions contre des crédits (FR19)
- **Selkies-GStreamer** — Logiciel open-source de streaming de bureau en WebRTC, accéléré par GPU, exécuté dans la VM invitée
- **Tailscale / Headscale** — Réseau maillé chiffré (WireGuard) gérant la traversée NAT et le contrôle d'accès
- **Cloudflare R2** — Stockage objet compatible S3 de Cloudflare, sans frais de téléchargement (egress); stockage plateforme au démarrage
- **Storj** — Réseau de stockage décentralisé, compatible S3, avec chiffrement et sharding côté client, payé en monnaie normale; ajouté en phase de croissance
- **rclone** — Outil de montage qui relie un stockage distant (R2, Storj, Google Drive, Dropbox) à un dossier de la VM, avec cache
