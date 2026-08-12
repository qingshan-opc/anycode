import { post } from "../http";

export type ReadFilePathResult = {
  path: string;
  name: string;
  /** text | binary | missing | dir */
  kind: "text" | "binary" | "missing" | "dir";
  content?: string;
  size_bytes: number;
  truncated: boolean;
};

/** 对话框粘贴本地文件路径时读取内容(后端有 4MB/10 个上限)。 */
export function readFilePaths(paths: string[]) {
  return post<{ files: ReadFilePathResult[] }>("/api/files/read-paths", { paths });
}
