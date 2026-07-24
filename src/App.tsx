import { useState } from "react";

function App() {
  const [count, setCount] = useState(0);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-zinc-950 text-white">
      <h1 className="mb-8 text-4xl font-bold">Bienvenido a Kue</h1>
      <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-8 text-center">
        <button
          className="rounded-lg bg-blue-600 px-6 py-3 text-lg font-medium transition-colors hover:bg-blue-500"
          onClick={() => setCount((c) => c + 1)}
        >
          Contador: {count}
        </button>
        <p className="mt-4 text-sm text-zinc-400">
          Haz clic para incrementar el contador
        </p>
      </div>
    </div>
  );
}

export default App;
