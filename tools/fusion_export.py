"""Fusion add-in script: export every visible body of the open design as its
own binary STL (one file per part, so the solver can split forces by part),
plus one STEP of the whole design, plus a ready-to-run case.toml.

Install: Fusion > Utilities > Add-Ins > Scripts and Add-Ins > + (My Scripts),
pick the folder holding this file, then Run. For a .f3z archive, open it in
Fusion first; this script exports whatever design is active.

Name wheel bodies or components with "wheel", "tyre", "tire" or "rim" in the
name: the solver spins those when you choose the moving-belt floor.

Not tested inside Fusion by Claude (no Fusion in its sandbox); report errors
from the Fusion text console and they can be fixed.
"""

import os
import re
import traceback

import adsk.core
import adsk.fusion


def _safe(name):
    return re.sub(r"[^A-Za-z0-9_-]+", "_", name).strip("_") or "part"


def run(context):
    app = adsk.core.Application.get()
    ui = app.userInterface
    try:
        design = adsk.fusion.Design.cast(app.activeProduct)
        if not design:
            ui.messageBox("Open a Fusion design first.")
            return
        dlg = ui.createFolderDialog()
        dlg.title = "Folder for the STL files and case.toml"
        if dlg.showDialog() != adsk.core.DialogResults.DialogOK:
            return
        out = dlg.folder
        em = design.exportManager
        root = design.rootComponent

        bodies = [(b.name, b) for b in root.bRepBodies if b.isVisible]
        for occ in root.allOccurrences:
            if not occ.isVisible:
                continue
            for b in occ.bRepBodies:
                if b.isVisible:
                    bodies.append((occ.name.split(":")[0] + "_" + b.name, b))

        files = []
        used = set()
        for name, body in bodies:
            base = _safe(name)
            n, i = base, 2
            while n in used:
                n, i = "%s_%d" % (base, i), i + 1
            used.add(n)
            path = os.path.join(out, n + ".stl")
            opts = em.createSTLExportOptions(body, path)
            opts.isBinaryFormat = True
            opts.meshRefinement = adsk.fusion.MeshRefinementSettings.MeshRefinementHigh
            em.execute(opts)
            files.append(n + ".stl")

        step_path = os.path.join(out, _safe(design.parentDocument.name) + ".step")
        em.execute(em.createSTEPExportOptions(step_path, root))

        lines = [
            'name = "%s"' % _safe(design.parentDocument.name),
            "speed_mps = 20.0",
            'quality = "quick"',
            'ground = "road"',
            'units = "auto"',
            'up_axis = "y"' if _y_up(app) else 'up_axis = "z"',
            "",
        ]
        for f in files:
            lines += ["[[geometry]]", 'path = "%s"' % f, ""]
        with open(os.path.join(out, "case.toml"), "w") as fh:
            fh.write("\n".join(lines))

        ui.messageBox(
            "Exported %d part(s), a STEP file and case.toml to\n%s\n\n"
            "Open case.toml in cfd-gui, or run: cfd run case.toml" % (len(files), out)
        )
    except Exception:
        ui.messageBox("Export failed:\n%s" % traceback.format_exc())


def _y_up(app):
    try:
        return app.preferences.generalPreferences.defaultModelingOrientation == (
            adsk.core.DefaultModelingOrientations.YUpModelingOrientation
        )
    except Exception:
        return False
