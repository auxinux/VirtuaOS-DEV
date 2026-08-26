# VirtuaOS — Workflow de publication (DEV → Public)

Ce document décrit le flux de publication entre les deux dépôts et les deux
canaux de mise à jour. Il est **interne** : il ne doit jamais être poussé dans
le dépôt public `VirtuaOS.git`.

## Les deux dépôts

| Dépôt | Visibilité | Rôle |
|-------|-----------|------|
| `Auxinux/VirtuaOS-DEV` | **Privé** (membres VIP) | Développement, préversions `-BETA` |
| `Auxinux/VirtuaOS` | **Public** (communauté) | Versions stables, code orienté communauté |

## Les deux canaux de mise à jour

| Canal | URL | Contenu |
|-------|-----|---------|
| Préversion (VIP) | `https://dep.auxinux.ca/VIRTUAOS/DEV/` | `-BETA`, accès anticipé |
| Stable (public) | `https://dep.auxinux.ca/VIRTUAOS/` | versions finalisées |

## Flux de publication

```
1. Développer dans VirtuaOS-DEV (privé)
        │
2. Bump version -BETA (VERSION, build.sh, branding.rs, …)
        │
3. Publier la préversion sur le canal DEV :
   → /var/www/dep.auxinux.ca/VIRTUAOS/DEV/<version>/
   → /var/www/dep.auxinux.ca/VIRTUAOS/DEV/release.md  (latest: <version>)
        │
4. Les VIP testent via :  vos os upgrade --channel dev
        │
5. Quand la version est jugée stable :
   → retirer le suffixe -BETA
   → copier le code nettoyé dans VirtuaOS.git (public)
   → publier sur le canal stable :
     /var/www/dep.auxinux.ca/VIRTUAOS/<version>/
     /var/www/dep.auxinux.ca/VIRTUAOS/release.md  (latest: <version>)
```

## Règle d'or pour le dépôt public

Le dépôt `VirtuaOS.git` est **100 % orienté communauté**. Avant chaque
publication, vérifier qu'il ne contient **aucune** référence interne :

- ❌ pas de script de push/déploiement (`prepareToVMbuilder.sh`, …)
- ❌ pas de chemin serveur (`/var/www/…`, `/home/debian`, …)
- ❌ pas de nom de machine interne (`virtua-repo`, `virtua-dev`, `debiandev`, …)
- ❌ pas de clé / token / secret / oauth
- ✅ `dep.auxinux.ca` est autorisé (URL publique de téléchargement)

## Synchronisation DEV → Public

Le dépôt public ne reçoit **que** les versions stables. Pour publier une
version stable :

```bash
# 1. Dans le clone DEV, exporter les fichiers trackés (sans .git ni target)
git archive --format=tar HEAD | tar -x -C /chemin/vers/clone-public/

# 2. Retirer les fichiers internes (prepareToVMbuilder.sh, …)
# 3. Nettoyer les références internes (chemins serveur, noms de machines)
# 4. Commiter + pousser vers VirtuaOS.git
```

Ne jamais pousser directement le `.git` du DEV vers le public : le public doit
avoir un historique propre, sans les commits de développement intermédiaires.
