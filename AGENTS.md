# Kue — OpenCode Agents

No hay agentes de OpenCode configurados actualmente en `.opencode/agents/`.

El proyecto no tiene flujos de automatización propios más allá de los skills instalados globalmente (listados en `skills-lock.json`):

| Skill | Fuente |
|---|---|
| accessibility | addyosmani/web-quality-skills |
| composition-patterns | vercel-labs/agent-skills |
| frontend-design | anthropics/skills |
| nodejs-backend-patterns | wshobson/agents |
| nodejs-best-practices | sickn33/antigravity-awesome-skills |
| react-best-practices | vercel-labs/agent-skills |
| seo | addyosmani/web-quality-skills |
| tailwind-css-patterns | giuseppe-trisciuoglio/developer-kit |
| tauri-v2 | nodnarbnitram/claude-code-extensions |
| typescript-advanced-types | wshobson/agents |
| vite | antfu/skills |

Para crear un nuevo agente, agrega un archivo `.opencode/agents/<nombre>/AGENT.md` siguiendo la [especificación de OpenCode](https://github.com/opencode-ai/agents).
