# Kue — OpenCode Agents

No OpenCode agents are currently configured in `.opencode/agents/`.

The project has no custom automation flows beyond globally installed skills (listed in `skills-lock.json`):

| Skill | Source |
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

To create a new agent, add a `.opencode/agents/<name>/AGENT.md` file following the [OpenCode agent specification](https://github.com/opencode-ai/agents).
