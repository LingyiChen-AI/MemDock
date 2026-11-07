import { EditorView } from "@codemirror/view";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { codeEntries, codeStatus, createCodeFile, readCode, saveCode, searchCode, trashCodeFile } from "@/api/workspaceCode";
import { renderApp } from "@/test/fixtures";
import { WorkspaceCodePanel } from "./WorkspaceCodePanel";
import { messageCodeLocation, toolCodeLocation, workspacePath } from "./workspaceCodeLinks";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "en" }),
}));

vi.mock("@/api/workspaceCode", () => ({
  codeEntries: vi.fn(),
  codeStatus: vi.fn(),
  createCodeFile: vi.fn(),
  searchCode: vi.fn(),
  readCode: vi.fn(),
  saveCode: vi.fn(),
  trashCodeFile: vi.fn(),
}));

it("resolves file references inside the current working directory", () => {
  expect(workspacePath("C:\\Work\\Demo", "c:\\work\\demo\\src\\main.rs")).toBe("src/main.rs");
  expect(workspacePath("/work/demo", "/work/demolition/main.rs")).toBeNull();
  expect(workspacePath("/work/demo", "../outside.rs")).toBeNull();
  expect(messageCodeLocation("src/main.rs:28:2")).toEqual({ path: "src/main.rs", line: 28 });
  expect(toolCodeLocation({ name: "Read", input: '{"file_path":"src/main.rs","offset":12}', output: null }))
    .toEqual({ path: "src/main.rs", line: 12 });
  expect(toolCodeLocation({ name: "apply_patch", input: "--- src/main.rs (update)\n@@ -10,2 +12,2 @@", output: null }))
    .toEqual({ path: "src/main.rs", line: 12 });
});

it("opens workspace code, retains a rejected draft, and saves through Ctrl+S", async () => {
  const getClientRects = vi.fn(() => []);
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: getClientRects });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockResolvedValue([{ name: "demo.ts", path: "demo.ts", directory: false }]);
  vi.mocked(readCode).mockResolvedValue({ kind: "text", text: "let n = 1;", revision: "rev-1", baseline: "let n = 0;" });
  vi.mocked(saveCode).mockRejectedValueOnce("The file changed on disk");
  const { container } = renderApp(<WorkspaceCodePanel cwd="/work/demo" base={null} shown request={null} />);
  const tree = await screen.findByRole("list", { name: "Files" });
  await userEvent.click(await within(tree).findByRole("button", { name: "demo.ts" }));
  expect(readCode).toHaveBeenCalledWith("/work/demo", "demo.ts", null);
  const editor = await waitFor(() => {
    const el = container.querySelector<HTMLElement>(".cm-content");
    expect(el).toHaveTextContent("let n = 1;");
    return el!;
  });
  expect(editor.querySelector(".cm-line")).toHaveClass("workspace-line-modified");
  const view = EditorView.findFromDOM(editor)!;
  act(() => view.dispatch({ changes: { from: view.state.doc.length, insert: " // edit" } }));
  await userEvent.click(screen.getByRole("button", { name: "Save (Ctrl+S)" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("changed on disk");
  expect(editor).toHaveTextContent("// edit");
  vi.mocked(saveCode).mockResolvedValueOnce("rev-2");
  fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
  await waitFor(() => expect(screen.getByRole("button", { name: "Save (Ctrl+S)" })).toBeDisabled());
  expect(saveCode).toHaveBeenLastCalledWith("/work/demo", "demo.ts", "rev-1", "let n = 1; // edit");
  await userEvent.click(screen.getByRole("button", { name: "Changes" }));
  expect(screen.getByText("let n = 0;")).toBeInTheDocument();
  expect(screen.getByText("let n = 1; // edit")).toBeInTheDocument();
});


it("marks added lines and the location of removed lines in the editable view", async () => {
  vi.mocked(codeStatus).mockResolvedValue({ "changes.ts": "M" });
  vi.mocked(codeEntries).mockResolvedValue([]);
  vi.mocked(readCode).mockResolvedValue({
    kind: "text",
    text: "start\nadded\nkeep\nend\n",
    baseline: "start\nkeep\ndeleted\nend\n",
    revision: "changes-1",
  });
  const { container } = renderApp(
    <WorkspaceCodePanel cwd="/work/changes" base={null} shown request={{ path: "changes.ts" }} />,
  );
  await waitFor(() => {
    const lines = Array.from(container.querySelectorAll(".cm-line"));
    expect(lines.find((line) => line.textContent === "added")).toHaveClass("workspace-line-added");
    expect(lines.find((line) => line.textContent === "end")).toHaveClass("workspace-line-deleted-before");
    expect(lines.find((line) => line.textContent === "keep")?.className).not.toContain("workspace-line-");
  });
});

it("uses a 60% split, remembers a resize, and creates and finds a file in the selected folder", async () => {
  const widthKey = "memdock.workspaceCode.widthPercent";
  localStorage.removeItem(widthKey);
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockImplementation(async (_, folder) => folder
    ? [{ name: "existing.ts", path: "src/existing.ts", directory: false }]
    : [{ name: "src", path: "src", directory: true }]);
  vi.mocked(createCodeFile).mockResolvedValue("src/new.ts");
  vi.mocked(readCode).mockResolvedValue({ kind: "text", text: "", revision: "new", baseline: "" });
  vi.mocked(searchCode).mockResolvedValue([{ path: "src/new.ts", line: null, text: null }]);
  try {
    renderApp(<WorkspaceCodePanel cwd="/work/create" base={null} shown request={null} />);
    const panel = await screen.findByRole("complementary", { name: "Code" });
    expect(panel.style.width).toBe("60%");
    fireEvent.keyDown(within(panel).getByRole("separator"), { key: "ArrowLeft" });
    expect(panel.style.width).toBe("63%");
    expect(localStorage.getItem(widthKey)).toBe("63");

    const folder = await within(panel).findByRole("button", { name: "src" });
    expect(folder).toHaveAttribute("aria-expanded", "false");
    expect(fireEvent.contextMenu(folder)).toBe(false);
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Copy Relative Path" })).toBeInTheDocument();
    await userEvent.click(within(menu).getByRole("menuitem", { name: "New File" }));
    expect(folder).toHaveAttribute("aria-expanded", "true");
    const files = within(panel).getByRole("list", { name: "src" });
    const input = within(files).getByRole("textbox", { name: "File name, Enter to create" });
    expect(files.firstElementChild).toContainElement(input);
    await waitFor(() => expect(input).toHaveFocus());
    await userEvent.click(folder);
    expect(input).not.toBeInTheDocument();
    expect(createCodeFile).not.toHaveBeenCalled();
    fireEvent.contextMenu(folder);
    await userEvent.click(await screen.findByRole("menuitem", { name: "New File" }));
    const reopened = within(panel).getByRole("list", { name: "src" });
    expect(await within(reopened).findByRole("button", { name: "existing.ts" })).toBeInTheDocument();
    await userEvent.type(within(reopened).getByRole("textbox"), "new.ts{Enter}");
    expect(createCodeFile).toHaveBeenCalledWith("/work/create", "src", "new.ts");
    expect(await within(panel).findByRole("button", { name: "new.ts", pressed: true })).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "p", ctrlKey: true });
    await userEvent.type(within(panel).getByRole("textbox", { name: "Find files" }), "new.ts");
    expect(await within(panel).findByRole("button", { name: "src/new.ts" })).toBeInTheDocument();
    expect(searchCode).toHaveBeenCalledWith("/work/create", "new.ts", false);
  } finally {
    localStorage.removeItem(widthKey);
  }
});

it("refreshes external edits and marks their ancestor folders when the window regains focus", async () => {
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockImplementation(async (_, folder) => folder
    ? [{ name: "external.ts", path: "src/external.ts", directory: false }]
    : [{ name: "src", path: "src", directory: true }]);
  vi.mocked(readCode).mockResolvedValue({ kind: "text", text: "before", baseline: "before", revision: "old" });
  const { container } = renderApp(<WorkspaceCodePanel cwd="/work/external" base={null} shown request={null} />);
  const tree = await screen.findByRole("list", { name: "Files" });
  const folder = await within(tree).findByRole("button", { name: "src" });
  await userEvent.click(folder);
  await userEvent.click(await within(tree).findByRole("button", { name: "external.ts" }));
  await waitFor(() => expect(container.querySelector(".cm-content")).toHaveTextContent("before"));

  vi.mocked(codeStatus).mockResolvedValue({ "src/external.ts": "M" });
  vi.mocked(readCode).mockResolvedValue({ kind: "text", text: "after", baseline: "before", revision: "new" });
  fireEvent.focus(window);

  await waitFor(() => {
    expect(within(tree).getByTitle("src/external.ts")).toHaveTextContent(/M$/);
    expect(within(tree).getByText("external.ts", { exact: true })).toHaveClass("text-amber-300");
    expect(within(folder).getByText("src")).toHaveClass("text-amber-300");
    expect(within(folder).getByTitle("Contains Git changes")).toBeInTheDocument();
    expect(container.querySelector(".workspace-line-modified")).toHaveTextContent("after");
  });
});

it("moves a clean file to the trash directly and removes its open tab", async () => {
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockResolvedValue([{ name: "delete.ts", path: "delete.ts", directory: false }]);
  vi.mocked(readCode).mockResolvedValue({ kind: "text", text: "", revision: "r1", baseline: null });
  vi.mocked(trashCodeFile).mockImplementation(async () => {
    vi.mocked(codeEntries).mockResolvedValue([]);
  });
  renderApp(<WorkspaceCodePanel cwd="/work/delete" base={null} shown request={null} />);
  const tree = await screen.findByRole("list", { name: "Files" });
  const entry = await within(tree).findByRole("button", { name: "delete.ts" });
  await userEvent.click(entry);
  fireEvent.contextMenu(entry);
  await userEvent.click(await screen.findByRole("menuitem", { name: "Delete File" }));
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  await waitFor(() => expect(screen.queryByRole("button", { name: "delete.ts" })).not.toBeInTheDocument());
  expect(trashCodeFile).toHaveBeenCalledWith("/work/delete", "delete.ts");
});

it("asks before discarding unsaved edits when deleting a file", async () => {
  vi.mocked(trashCodeFile).mockClear();
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockResolvedValue([{ name: "draft.ts", path: "draft.ts", directory: false }]);
  vi.mocked(readCode).mockResolvedValue({ kind: "text", text: "", revision: "r1", baseline: null });
  const { container } = renderApp(<WorkspaceCodePanel cwd="/work/delete-draft" base={null} shown request={null} />);
  const tree = await screen.findByRole("list", { name: "Files" });
  const entry = await within(tree).findByRole("button", { name: "draft.ts" });
  await userEvent.click(entry);
  const editor = await waitFor(() => {
    const view = EditorView.findFromDOM(container.querySelector(".cm-content")!);
    expect(view).not.toBeNull();
    return view!;
  });
  act(() => editor.dispatch({ changes: { from: 0, insert: "unsaved" } }));
  fireEvent.contextMenu(entry);
  await userEvent.click(await screen.findByRole("menuitem", { name: "Delete File" }));
  const dialog = await screen.findByRole("dialog", { name: "Delete File" });
  expect(dialog).toHaveTextContent("unsaved changes");
  expect(trashCodeFile).not.toHaveBeenCalled();
  await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(container.querySelector(".cm-content")).toHaveTextContent("unsaved");
});

it("opens untitled tabs without writing to disk and uses the selected folder on first save", async () => {
  vi.mocked(createCodeFile).mockClear().mockImplementation(async (_, folder, name) => folder ? folder + "/" + name : name);
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockImplementation(async (_, folder) => folder ? [] : [{ name: "src", path: "src", directory: true }]);
  vi.mocked(readCode).mockImplementation(async (_, path) => ({
    kind: "text", text: path === "root.ts" ? "draft contents" : "", revision: "saved", baseline: "",
  }));
  const { container } = renderApp(<WorkspaceCodePanel cwd="/work/tab-create" base={null} shown request={{ path: "src/existing.ts" }} />);
  const blank = await screen.findByRole("button", { name: "Double-click to create a file" });
  const tree = await screen.findByRole("list", { name: "Files" });

  await userEvent.click(blank);
  expect(screen.queryByRole("button", { name: /Untitled-/ })).not.toBeInTheDocument();
  await userEvent.dblClick(blank);
  await screen.findByRole("button", { name: /Untitled-/, pressed: true });
  expect(screen.queryByRole("textbox", { name: "File name, Enter to create" })).not.toBeInTheDocument();
  expect(createCodeFile).not.toHaveBeenCalled();
  const editor = await waitFor(() => {
    const view = EditorView.findFromDOM(container.querySelector(".cm-content")!);
    expect(view).not.toBeNull();
    return view!;
  });
  act(() => editor.dispatch({ changes: { from: 0, insert: "draft contents" } }));
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  let dialog = await screen.findByRole("dialog", { name: "Save As" });
  await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(container.querySelector(".cm-content")).toHaveTextContent("draft contents");
  expect(createCodeFile).not.toHaveBeenCalled();
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  dialog = await screen.findByRole("dialog", { name: "Save As" });
  await userEvent.type(within(dialog).getByRole("textbox", { name: "File name" }), "root.ts{Enter}");
  expect(createCodeFile).toHaveBeenLastCalledWith("/work/tab-create", "", "root.ts", "draft contents");
  await screen.findByRole("button", { name: "root.ts", pressed: true });
  expect(screen.queryByRole("button", { name: /Untitled-/ })).not.toBeInTheDocument();

  await userEvent.dblClick(blank);
  await screen.findByRole("button", { name: /Untitled-/, pressed: true });
  await userEvent.click(await within(tree).findByRole("button", { name: "src" }));
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  dialog = await screen.findByRole("dialog", { name: "Save As" });
  await userEvent.type(within(dialog).getByRole("textbox", { name: "File name" }), "child.ts{Enter}");
  expect(createCodeFile).toHaveBeenLastCalledWith("/work/tab-create", "src", "child.ts", "");
});

it("opens code-specific context menus on tabs and inside the editor", async () => {
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockResolvedValue([]);
  vi.mocked(readCode).mockResolvedValue({ kind: "text", text: "const n = 1;", revision: "r1", baseline: null });
  const { container } = renderApp(<WorkspaceCodePanel cwd="/work/menus" base={null} shown request={{ path: "menu.ts" }} />);
  const tab = await screen.findByRole("button", { name: "menu.ts", pressed: true });
  const content = await waitFor(() => {
    const element = container.querySelector<HTMLElement>(".cm-content");
    expect(element).toHaveTextContent("const n = 1;");
    return element!;
  });
  expect(fireEvent.contextMenu(content)).toBe(false);
  await userEvent.click(await screen.findByRole("menuitem", { name: "Select All" }));
  const editor = EditorView.findFromDOM(content)!;
  expect(editor.state.selection.main.to).toBe(editor.state.doc.length);

  expect(fireEvent.contextMenu(tab)).toBe(false);
  const menu = await screen.findByRole("menu");
  expect(within(menu).getByRole("menuitem", { name: "Copy Relative Path" })).toBeInTheDocument();
  await userEvent.click(within(menu).getByRole("menuitem", { name: "Close All" }));
  expect(tab).not.toBeInTheDocument();
});

it("reuses untitled numbers and prompts inside the panel before closing an edited tab", async () => {
  vi.mocked(createCodeFile).mockClear().mockResolvedValue("saved.ts");
  vi.mocked(codeStatus).mockResolvedValue({});
  vi.mocked(codeEntries).mockResolvedValue([]);
  const { container } = renderApp(<WorkspaceCodePanel cwd="/work/untitled-close" base={null} shown request={null} />);
  const panel = await screen.findByRole("complementary", { name: "Code" });
  const blank = within(panel).getByRole("button", { name: "Double-click to create a file" });

  await userEvent.dblClick(blank);
  await userEvent.dblClick(blank);
  expect(within(panel).getByRole("button", { name: "Untitled-2", pressed: true })).toBeInTheDocument();
  await userEvent.click(within(panel).getByRole("button", { name: "Close Untitled-1" }));
  expect(within(panel).queryByRole("dialog")).not.toBeInTheDocument();
  await userEvent.dblClick(blank);
  expect(within(panel).getByRole("button", { name: "Untitled-1", pressed: true })).toBeInTheDocument();

  const editor = await waitFor(() => {
    const content = container.querySelector<HTMLElement>(".cm-content");
    expect(content).not.toBeNull();
    return EditorView.findFromDOM(content!)!;
  });
  act(() => editor.dispatch({ changes: { from: 0, insert: "unsaved contents" } }));
  const close = within(panel).getByRole("button", { name: "Close Untitled-1" });
  await userEvent.click(close);
  let prompt = within(panel).getByRole("dialog", { name: "Save changes before closing?" });
  await userEvent.click(within(prompt).getByRole("button", { name: "Cancel" }));
  expect(container.querySelector(".cm-content")).toHaveTextContent("unsaved contents");
  await userEvent.click(close);
  prompt = within(panel).getByRole("dialog", { name: "Save changes before closing?" });
  await userEvent.click(within(prompt).getByRole("button", { name: "Save" }));
  let save = within(panel).getByRole("dialog", { name: "Save As" });
  await userEvent.click(within(save).getByRole("button", { name: "Cancel" }));
  expect(within(panel).queryByRole("dialog")).not.toBeInTheDocument();
  expect(container.querySelector(".cm-content")).toHaveTextContent("unsaved contents");
  await userEvent.click(close);
  await userEvent.click(within(within(panel).getByRole("dialog")).getByRole("button", { name: "Save" }));
  save = within(panel).getByRole("dialog", { name: "Save As" });
  await userEvent.type(within(save).getByRole("textbox"), "saved.ts{Enter}");
  await waitFor(() => expect(within(panel).queryByRole("button", { name: "Untitled-1" })).not.toBeInTheDocument());
  expect(createCodeFile).toHaveBeenCalledWith("/work/untitled-close", "", "saved.ts", "unsaved contents");
  expect(within(panel).queryByRole("button", { name: "saved.ts" })).not.toBeInTheDocument();
  await userEvent.dblClick(blank);
  expect(within(panel).getByRole("button", { name: "Untitled-1", pressed: true })).toBeInTheDocument();
});
