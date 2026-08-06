#!/usr/bin/env python3
import re
import sys
import xml.etree.ElementTree as ET


def center(node):
    match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.attrib.get("bounds", ""))
    if not match:
        return None
    left, top, right, bottom = map(int, match.groups())
    return (left + right) // 2, (top + bottom) // 2


root = ET.parse(sys.argv[1]).getroot()
attribute = "text" if sys.argv[2] == "text" else "content-desc"
parents = {child: parent for parent in root.iter() for child in parent}
fallback = None
for node in root.iter("node"):
    if node.attrib.get(attribute) != sys.argv[3]:
        continue
    target = node
    while target is not None and target.attrib.get("clickable") != "true":
        target = parents.get(target)
    point = center(target or node)
    if point is None:
        continue
    if target is not None:
        print(*point)
        raise SystemExit(0)
    fallback = fallback or point
if fallback is not None:
    print(*fallback)
    raise SystemExit(0)
raise SystemExit(1)
