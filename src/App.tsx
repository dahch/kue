import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface IndexSummary {
  folder: string;
  files_indexed: number;
  chunks_created: number;
  error_count: number;
}

interface SearchResult {
  id: number;
  document_id: number;
  text: string;
  chunk_index: number;
  tag: string | null;
  metric: string | null;
  score: number;
}

function App() {
  const [indexResult, setIndexResult] = useState<string>("");
  const [query, setQuery] = useState("");
  const [searchResults, setSearchResults] = useState<string>("");

  const handleIndex = async () => {
    setIndexResult("Indexando...");
    try {
      const path = prompt("Ruta de la carpeta a indexar:", "/tmp/kue-index-test");
      if (!path) return;
      const result = await invoke<IndexSummary>("index_folder_cmd", { path });
      setIndexResult(
        `OK: ${result.files_indexed} archivos, ${result.chunks_created} chunks`
      );
    } catch (e) {
      setIndexResult(`Error: ${e}`);
    }
  };

  const handleSearch = async () => {
    if (!query.trim()) return;
    setSearchResults("Buscando...");
    try {
      const results = await invoke<SearchResult[]>("search_context", {
        query,
        top_k: 5,
      });
      if (results.length === 0) {
        setSearchResults("Sin resultados.");
        return;
      }
      setSearchResults(
        results
          .map(
            (r, i) =>
              `#${i + 1} (score: ${r.score.toFixed(3)}): ${r.text.slice(0, 120)}...`
          )
          .join("\n")
      );
    } catch (e) {
      setSearchResults(`Error: ${e}`);
    }
  };

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-zinc-950 p-8 text-white">
      <h1 className="mb-8 text-4xl font-bold">Bienvenido a Kue</h1>

      {/* Debug: Index Folder */}
      <div className="mb-6 w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-6">
        <h2 className="mb-3 text-lg font-semibold text-blue-400">
          Debug: Indexar carpeta
        </h2>
        <button
          className="rounded-lg bg-blue-600 px-6 py-2 font-medium transition-colors hover:bg-blue-500"
          onClick={handleIndex}
        >
          Indexar carpeta
        </button>
        {indexResult && (
          <pre className="mt-3 whitespace-pre-wrap text-sm text-zinc-300">
            {indexResult}
          </pre>
        )}
      </div>

      {/* Debug: Search */}
      <div className="mb-6 w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-6">
        <h2 className="mb-3 text-lg font-semibold text-blue-400">
          Debug: Buscar contexto
        </h2>
        <div className="flex gap-2">
          <input
            className="flex-1 rounded-lg border border-zinc-600 bg-zinc-800 px-3 py-2 text-sm text-white placeholder-zinc-500"
            placeholder="término de búsqueda..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSearch()}
          />
          <button
            className="rounded-lg bg-emerald-600 px-4 py-2 font-medium transition-colors hover:bg-emerald-500"
            onClick={handleSearch}
          >
            Buscar
          </button>
        </div>
        {searchResults && (
          <pre className="mt-3 whitespace-pre-wrap text-sm text-zinc-300">
            {searchResults}
          </pre>
        )}
      </div>
    </div>
  );
}

export default App;
