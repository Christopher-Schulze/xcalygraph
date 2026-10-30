import openRepo, { Repo, MemoryRepo } from "./store/repo";
import * as store from "@/store";
import { format } from "../lib/format.js";
import React from "react";

export class Service {
  constructor(private repo: Repo) {}

  find(id: string): string {
    const row = this.repo.get(id);
    return format(row?.id ?? process.env.DEFAULT_ID ?? "");
  }
}

export function makeService(): Service {
  const repo = openRepo();
  const memory = new MemoryRepo();
  memory.get("x");
  const other = new store.MemoryRepo();
  return new Service(repo);
}
