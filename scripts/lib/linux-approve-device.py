#!/usr/bin/env python3

import pyatspi

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


def find_approval_dialog():
    desktop = pyatspi.Registry.getDesktop(0)
    for node in descendants(desktop):
        try:
            if node.name == APPROVAL_DIALOG_TITLE:
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


dialog = find_approval_dialog()
button = find_approve_button(dialog) if dialog is not None else None
raise SystemExit(0 if button is not None and activate(button) else 1)
