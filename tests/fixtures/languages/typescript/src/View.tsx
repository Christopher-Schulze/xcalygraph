import { makeService } from "../service";

export function Badge(props: { label: string }) {
  return <span>{props.label}</span>;
}

export function View() {
  const service = makeService();
  return <Badge label={service.find("1")} />;
}
