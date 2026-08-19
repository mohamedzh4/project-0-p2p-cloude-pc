# Cahier des Charges — Distributed Compute Lab Platform

## 1. Contexte

Traditional cloud computing routes every workload through centralized datacenter servers. This project instead lets individuals with underused high-performance PCs share that idle capacity with other users, over a peer network, accessed as an on-demand virtual desktop through a browser — a "lab" the user can log into rather than a physical machine they own.

## 2. Objectifs du Projet

- Let a user with a low-spec PC access a high-performance virtual desktop, matched automatically to the best available peer host
- Let a host monetize idle CPU/RAM/GPU capacity with near-zero manual effort
- Guarantee that neither side compromises the other: the host never sees the user's data, and the host retains nothing once a session ends
- Deliver a session that feels indistinguishable from working on the physical machine, for both office-grade and GPU-heavy workloads

## 3. Parties Prenantes

| Rôle | Description |
|---|---|
| **Utilisateur** | Rents a virtual desktop session; brings an account, optional persistent files, and a task ("what do you want to do today") |
| **Host / Provider** | Shares a declared slice of their PC's CPU, RAM, and optionally GPU; runs the Host Agent |
| **Plateforme (opérateur)** | Runs the Matchmaker, billing, base-image pipeline, and network infrastructure — the only centralized piece |

## 4. Périmètre

### Inclus dans le MVP
- Standard Lab (Firecracker + Kasm Workspaces)
- Pro/Gamer Lab (QEMU/KVM + VFIO GPU passthrough + Sunshine/Moonlight)
- Web app for users, tray app for hosts
- Matchmaker: auth, intent capture, matching, session lifecycle, billing
- Encrypted persistent storage (Storj + rclone)
- Secure mesh networking (Tailscale/Headscale)

### Exclu du MVP (phases futures)
- Confidential computing tier (SEV-SNP/TDX-aware routing)
- Multi-region redundancy for the control plane
- Native mobile clients
- Enterprise SSO / team accounts

## 5. Besoins Fonctionnels

### 5.1 Côté Utilisateur
- FR1 — Create an account and log in (resolved internally to `User ID X`)
- FR2 — Be asked "What do you want to do in your lab today?" and have the answer routed to a lab tier
- FR3 — Be automatically matched to the most suitable available host
- FR4 — Access previously stored files inside the session, transparently and encrypted
- FR5 — Work in a live streamed desktop session with no perceptible lag for the Standard tier
- FR6 — Log out with a guaranteed final save; see a "saved as of HH:MM" indicator during the session
- FR7 — View session history and billing/credits

### 5.2 Côté Host/Provider
- FR8 — Register and declare exactly how much RAM, CPU, and GPU to share, and on what schedule
- FR9 — Have the Host Agent auto-join the network mesh and download the correct signed base image without manual steps
- FR10 — See a simple status view (idle / in-session / earnings) with nothing to operate mid-session
- FR11 — Pause or stop sharing at any time

### 5.3 Côté Plateforme (Matchmaker)
- FR12 — Track real-time host inventory: resources, load, reliability score
- FR13 — Match on tier fit, latency, and reliability, in that priority order for the Pro tier
- FR14 — Provision, monitor (heartbeat), and tear down sessions automatically
- FR15 — Re-match a user to a new host within the bounded data-loss window if a host disconnects
- FR16 — Meter usage and settle billing/payouts

## 6. Besoins Non-Fonctionnels

| Catégorie | Exigence |
|---|---|
| Performance | Standard Lab sessions must feel locally responsive for office/browsing/coding workloads; Pro/Gamer sessions need latency low enough for real-time video/gaming (target sub-50ms round trip where possible) |
| Sécurité | Zero data retention on host after logout; encryption at rest and in transit; signed, verified base images; per-session network isolation |
| Fiabilité | Bounded data-loss window on host disconnect (event-triggered sync, not just the 5–10 min timer) |
| Scalabilité | Adding hosts or users must not require redesigning the four-plane architecture |
| Disponibilité | The Matchmaker (control plane) is the one centralized piece and needs a real uptime target, even though compute is decentralized |
| Utilisabilité | Zero install for the Standard Lab (browser only); minimal friction accepted for the Pro/Gamer Lab (native client tolerable) |

## 7. Architecture Technique (Résumé)

| Plan | Rôle | Outils |
|---|---|---|
| Contrôle | Auth, matching, billing | Matchmaker (custom) |
| Calcul | Exécute les sessions | Firecracker (Standard) / QEMU+KVM+VFIO (Pro) |
| Réseau | Connecte user↔VM↔storage | Tailscale / Headscale |
| Stockage | Persistance des données | Storj + rclone |

*Voir le document "Distributed Compute Lab — Complete Architecture Specification" pour le détail technique complet, incluant la table des 11 vulnérabilités identifiées et leurs correctifs.*

## 8. Contraintes Techniques et Légales

- Pro/Gamer hosts require an IOMMU-capable CPU/motherboard (VT-d / AMD-Vi)
- Kernel-level anti-cheat (EAC, BattlEye, Vanguard) blocks some titles by vendor policy — not something the architecture can route around
- Consumer GPU driver EULAs (NVIDIA/AMD) may restrict commercial "cloud gaming"-style use — requires legal review before commercial launch
- Guest network egress must not exit through the host's raw ISP connection — routed through a platform-controlled exit node instead, to avoid exposing hosts to legal liability for a user's traffic
- Terms of Service, acceptable-use policy, and host indemnification require legal review before public launch

## 9. Stratégie de Développement — Par Où Commencer

The Matchmaker (backend) is conventional, low-risk engineering. The compute pipeline — Firecracker+Kasm feeling native, VFIO+Sunshine hitting playable latency — is where the real technical risk lives. Development should follow risk first, not layer first:

1. Manually validate one Firecracker+Kasm session and one QEMU+VFIO+Sunshine session — no server, no UI — to confirm the core technical bet holds
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
| Phase 1 | MVP Standard Lab, région unique, hosts approuvés manuellement |
| Phase 2 | Matchmaker complet (matching + billing), onboarding host automatisé |
| Phase 3 | Ajout du Pro/Gamer Lab, matching sensible à la latence |
| Phase 4 | Durcissement sécurité (signature d'image, exit nodes, scoring de fiabilité), tier confidential computing si demandé par le marché |

## 12. Critères d'Acceptation

- Un utilisateur obtient une session Standard Lab utilisable en moins de quelques secondes après sélection
- Une déconnexion host en cours de session entraîne une perte de données bornée (quelques minutes maximum) et un re-matching automatique
- Aucune trace des données utilisateur ne subsiste sur le disque physique du host après une session
- Le host n'a aucune action à effectuer pendant une session active, hormis rester allumé et connecté

## 13. Risques Principaux

| Risque | Impact | Mitigation |
|---|---|---|
| Le trafic sortant de la VM expose l'IP du host | Exposition légale d'un hôte innocent | Routage via exit node contrôlé par la plateforme |
| Absence de confidential computing par défaut | Le host pourrait théoriquement inspecter la mémoire d'une VM | Router les sessions sensibles vers des hosts SEV-SNP/TDX quand disponibles |
| Incompatibilité anti-cheat sur certains jeux | Une partie du catalogue gaming ne fonctionnera pas | Publier une liste de compatibilité, ne pas tenter de contourner les politiques éditeurs |
| Licences des pilotes GPU grand public | Restriction possible d'usage commercial | Revue juridique avant lancement commercial |

## 14. Glossaire

- **Matchmaker** — Service central qui authentifie les utilisateurs, fait correspondre demande et host, et pilote le cycle de vie des sessions
- **Host Agent** — Application légère installée chez le host, qui rejoint le réseau maillé et déclenche le provisioning des VMs
- **Base Image** — Image système standby, signée, prête à démarrer instantanément sur un host
- **Firecracker** — MicroVM légère basée sur KVM, sans passthrough PCI, utilisée pour le Standard Lab
- **VFIO** — Mécanisme du noyau Linux permettant le passthrough direct d'un GPU physique vers une VM
- **Tailscale / Headscale** — Réseau maillé chiffré (WireGuard) gérant la traversée NAT et le contrôle d'accès
- **Storj** — Réseau de stockage décentralisé, compatible S3, avec chiffrement et sharding côté client
- **rclone** — Outil de montage qui relie un stockage distant (Storj) à un système de fichiers local, avec cache
