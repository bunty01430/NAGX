use std::{fs, path::PathBuf};

fn replace_once(source: &mut String, old: &str, new: &str, label: &str) {
    let count = source.matches(old).count();
    assert_eq!(count, 1, "expected exactly one {label} anchor, found {count}");
    *source = source.replacen(old, new, 1);
}

fn main() {
    println!("cargo:rerun-if-changed=ui/main.slint");

    let source = fs::read_to_string("ui/main.slint").expect("failed to read Slint UI");
    let mut ui = source;

    replace_once(
        &mut ui,
        "    in-out property <bool> terminal-focused: true;\n",
        "    in-out property <bool> terminal-focused: true;\n    in-out property <int> terminal-drag-origin-x: 0;\n    in-out property <int> terminal-drag-origin-y: 0;\n    in-out property <int> terminal-resize-origin-width: 860;\n    in-out property <int> terminal-resize-origin-height: 520;\n",
        "terminal interaction properties",
    );

    replace_once(
        &mut ui,
        "                    TouchArea { x: 0px; y: 0px; width: parent.width - 84px; height: 28px; clicked => { root.terminal-focused = true; terminal-input.focus(); } }\n",
        "                    TouchArea {\n                        x: 0px; y: 0px; width: parent.width - 84px; height: 28px;\n                        pointer-event(event) => {\n                            root.terminal-focused = true;\n                            terminal-input.focus();\n                            if event.button == PointerEventButton.left {\n                                if event.kind == PointerEventKind.down && !root.terminal-maximized {\n                                    root.terminal-drag-origin-x = root.terminal-x;\n                                    root.terminal-drag-origin-y = root.terminal-y;\n                                } else if event.kind == PointerEventKind.move && !root.terminal-maximized && self.pressed {\n                                    let dx = (self.mouse-x - self.pressed-x) / 1px;\n                                    let dy = (self.mouse-y - self.pressed-y) / 1px;\n                                    root.terminal-x = floor(max(0, min(root.terminal-drag-origin-x + dx, root.width / 1px - 340)));\n                                    root.terminal-y = floor(max(70, min(root.terminal-drag-origin-y + dy, root.height / 1px - 60)));\n                                }\n                            }\n                            return accept;\n                        }\n                    }\n",
        "terminal titlebar interaction",
    );

    replace_once(
        &mut ui,
        "                        TouchArea {\n                            visible: !(root.active-terminal == 1 ? root.terminal-1-mouse-reporting : root.active-terminal == 2 ? root.terminal-2-mouse-reporting : root.terminal-3-mouse-reporting);\n                            enabled: self.visible;\n                            mouse-cursor: text;\n                            clicked => { root.terminal-focused = true; terminal-input.focus(); }\n                        }\n",
        "                        terminal-selection := TextInput {\n                            visible: !(root.active-terminal == 1 ? root.terminal-1-mouse-reporting : root.active-terminal == 2 ? root.terminal-2-mouse-reporting : root.terminal-3-mouse-reporting);\n                            enabled: self.visible;\n                            width: parent.width;\n                            height: parent.height;\n                            text: root.active-terminal == 1 ? root.terminal-1-plain : root.active-terminal == 2 ? root.terminal-2-plain : root.terminal-3-plain;\n                            read-only: true;\n                            single-line: false;\n                            wrap: no-wrap;\n                            font-family: \"Cascadia Mono\";\n                            font-size: 13px;\n                            color: #00000000;\n                            selection-background-color: #31546a;\n                            selection-foreground-color: #ffffff;\n                            text-cursor-width: 0px;\n                            key-pressed(event) => {\n                                if event.modifiers.alt {\n                                    if event.text == \"1\" { root.select-terminal-ui(1); return accept; }\n                                    if event.text == \"2\" { root.select-terminal-ui(2); return accept; }\n                                    if event.text == \"3\" { root.select-terminal-ui(3); return accept; }\n                                    if event.text == \"Tab\" { root.select-terminal-ui(root.active-terminal == 3 ? 1 : root.active-terminal + 1); return accept; }\n                                }\n                                if event.modifiers.control && event.modifiers.shift {\n                                    if event.text == \"v\" || event.text == \"V\" { root.terminal-paste(root.active-terminal); return accept; }\n                                    if event.text == \"a\" || event.text == \"A\" { terminal-selection.select-all(); return accept; }\n                                    if event.text == \"c\" || event.text == \"C\" { terminal-selection.copy(); return accept; }\n                                }\n                                root.terminal-key(root.active-terminal, event.text, event.modifiers.control, event.modifiers.alt, event.modifiers.shift);\n                                return accept;\n                            }\n                        }\n",
        "terminal local selection layer",
    );

    replace_once(
        &mut ui,
        "                    Rectangle { x: 10px; y: 8px; width: parent.width - 20px; height: parent.height - 16px; clip: true;\n",
        "                    Rectangle {\n                        x: 10px; y: 8px; width: parent.width - 20px; height: parent.height - 16px; clip: true;\n                        changed width => {\n                            root.terminal-resize(\n                                root.active-terminal,\n                                max(1, floor(self.width / 8.45px)),\n                                max(1, floor(self.height / 17px)),\n                                max(1, round(self.width / 1px)),\n                                max(1, round(self.height / 1px))\n                            );\n                        }\n                        changed height => {\n                            root.terminal-resize(\n                                root.active-terminal,\n                                max(1, floor(self.width / 8.45px)),\n                                max(1, floor(self.height / 17px)),\n                                max(1, round(self.width / 1px)),\n                                max(1, round(self.height / 1px))\n                            );\n                        }\n",
        "terminal viewport resize callbacks",
    );

    replace_once(
        &mut ui,
        "                }\n            }\n            if root.terminal-minimized : Rectangle { x: min(root.terminal-x * 1px, parent.width - 330px);",
        "                }\n                TouchArea {\n                    visible: !root.terminal-maximized;\n                    enabled: self.visible;\n                    x: parent.width - 18px; y: parent.height - 18px; width: 18px; height: 18px;\n                    mouse-cursor: se-resize;\n                    pointer-event(event) => {\n                        if event.button == PointerEventButton.left && event.kind == PointerEventKind.down {\n                            root.terminal-resize-origin-width = root.terminal-width;\n                            root.terminal-resize-origin-height = root.terminal-height;\n                        } else if event.button == PointerEventButton.left && event.kind == PointerEventKind.move && self.pressed {\n                            let dx = (self.mouse-x - self.pressed-x) / 1px;\n                            let dy = (self.mouse-y - self.pressed-y) / 1px;\n                            root.terminal-width = floor(max(520, min(root.terminal-resize-origin-width + dx, root.width / 1px - root.terminal-x - 20)));\n                            root.terminal-height = floor(max(300, min(root.terminal-resize-origin-height + dy, root.height / 1px - root.terminal-y - 20)));\n                        }\n                        return accept;\n                    }\n                }\n            }\n            if root.terminal-minimized : Rectangle { x: min(root.terminal-x * 1px, parent.width - 330px);",
        "terminal resize handle",
    );

    let mut out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    out.push("main.generated.slint");
    fs::write(&out, ui).expect("failed to write generated Slint UI");
    slint_build::compile(out.to_str().expect("generated UI path is not UTF-8"))
        .expect("failed to compile generated Slint UI");
}
