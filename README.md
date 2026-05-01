# lojix-cli-v2

Forked development repo for the next generation of `lojix-cli`.
This starts from the current working deploy orchestrator, but it is
the place where the Nota-native CLI, request files, and home deploy
work land without destabilizing Li's live tool.

```
lojix-cli-v2 deploy --cluster goldragon --node tiger --source ./datom.nota
lojix-cli-v2 build  --cluster goldragon --node tiger --source ./datom.nota
lojix-cli-v2 eval   --cluster goldragon --node tiger --source ./datom.nota
```

The current design target is documented in
`~/git/CriomOS/reports/0038-lojix-local-config-and-home-deploy-design.md`.

For repo rules, read `AGENTS.md`. For the repo's role, read
`ARCHITECTURE.md`.
