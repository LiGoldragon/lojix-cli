# lojix-cli

Forked development repo for the next generation of `lojix-cli`.
This starts from the current working deploy orchestrator, but it is
the place where the Nota-native CLI, request files, and home deploy
work land without destabilizing Li's live tool.

```
lojix-cli '(FullOs goldragon tiger "./datom.nota" "github:LiGoldragon/CriomOS/<rev>" Boot None)'
lojix-cli '(OsOnly goldragon tiger "./datom.nota" "github:LiGoldragon/CriomOS/<rev>" Boot None)'
lojix-cli '(HomeOnly goldragon tiger li "./datom.nota" "github:LiGoldragon/CriomOS/<rev>" Profile None)'
lojix-cli ./request.nota
lojix-cli
```

The current design target is documented in
`~/git/CriomOS/reports/0038-lojix-local-config-and-home-deploy-design.md`.

For repo rules, read `AGENTS.md`. For the repo's role, read
`ARCHITECTURE.md`.
