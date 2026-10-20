#!/usr/bin/env python3

import pyatspi
import sys

APPROVAL_DIALOG_TITLE = "Approve this device?"
APPROVAL_BUTTON_NAME = "Approve"
ACTIVATE_ACTIONS = {"activate", "click", "press"}
DIALOG_ROLES = {pyatspi.ROLE_ALERT, pyatspi.ROLE_DIALOG}


def descendants(root):
    pending = [root]
    while pending:
        node = pending.pop()
        yield node
        try:
            pending.extend(node)
        except (LookupError, RuntimeError):
            continue


def enclosing_dialog(node):
    current = node
    while current is not None:
        try:
            if current.getRole() in DIALOG_ROLES:
                return current
            current = current.parent
        except (LookupError, RuntimeError):
            return None
    return None


def app_windows(app_pid):
    desktop = pyatspi.Registry.getDesktop(0)
    for node in descendants(desktop):
        if (node.name == "Iris Drive" and node.getRole() == pyatspi.ROLE_FRAME
                and node.get_process_id() == app_pid):
            yield node


def approval_completed(app_pid):
    return any(node.name == "Device approved"
               for window in app_windows(app_pid) for node in descendants(window))


def find_approval_dialog(app_pid):
    desktop = pyatspi.Registry.getDesktop(0)
    for node in descendants(desktop):
        try:
            if node.name == APPROVAL_DIALOG_TITLE and node.get_process_id() == app_pid:
                dialog = enclosing_dialog(node)
                if dialog is not None:
                    return dialog
        except (LookupError, RuntimeError):
            continue
    return None


def find_approve_button(dialog):
    for node in descendants(dialog):
        try:
            if node.name == APPROVAL_BUTTON_NAME and node.getRole() == pyatspi.ROLE_PUSH_BUTTON:
                return node
        except (LookupError, RuntimeError):
            continue
    return None


def activate(button):
    try:
        actions = button.queryAction()
        for index in range(actions.nActions):
            if actions.getName(index) in ACTIVATE_ACTIONS:
                print("IRIS_DRIVE_DESKTOP_GUI_APPROVAL_STARTED=1", flush=True)
                return bool(actions.doAction(index))
    except (LookupError, RuntimeError):
        pass
    return False


if len(sys.argv) != 3 or sys.argv[1] not in ("--activate", "--completed"):
    raise SystemExit("expected --activate or --completed followed by the GTK process id")
app_pid = int(sys.argv[2])
if app_pid <= 0:
    raise SystemExit(1)
if sys.argv[1] == "--completed":
    raise SystemExit(0 if approval_completed(app_pid) else 1)
dialog = find_approval_dialog(app_pid)
button = find_approve_button(dialog) if dialog is not None else None
raise SystemExit(0 if button is not None and activate(button) else 1)
