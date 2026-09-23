import { useHive } from "./store";

export function App() {
  const status = useHive((s) => s.connection.status);
  return <main data-connection={status}>Hive</main>;
}
