import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

interface HintPayload {
  text: string;
  type: string;
  session_id: string;
}

function Overlay() {
  const [hint, setHint] = useState<HintPayload | null>(null);
  const [visible, setVisible] = useState(false);
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    const unlisten = listen<HintPayload>("new-hint", (event) => {
      setHint(event.payload);
      setVisible(true);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setVisible(false), 3000);
    });
    return () => {
      unlisten.then((fn) => fn());
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  return (
    <div
      className="fixed inset-0 flex items-start justify-center pt-8 transition-opacity duration-500"
      style={{ opacity: visible ? 1 : 0 }}
    >
      <div className="rounded-xl bg-black/60 px-8 py-5 text-center text-xl font-medium leading-relaxed text-white shadow-2xl backdrop-blur-md">
        {hint?.text ?? ""}
      </div>
    </div>
  );
}

export default Overlay;
