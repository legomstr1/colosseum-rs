"""Coordinate descent over tuned-1 params using the sweep harness (a copy of
tuning/live-input/descend.py with the tuned-1 parameter space). `partner_gain`
stays at 0: offers must never leave the partner worse off.

Each batch tries one step up and one down for a group of params against the
current best on the same seeds; the best variant wins if it beats the current
one by `MARGIN` win-rate points and doesn't place worse. Writes progress to
descend.log and the current best to best.txt.

python tuning/tuned-1/descend.py [minutes] [games] [field]
SWEEP_EXE overrides the harness binary (so a rebuild mid-run is safe).
"""
import subprocess, sys, time, re, os, random

HERE = os.path.dirname(os.path.abspath(__file__))
EXE = os.environ.get("SWEEP_EXE") or os.path.join(HERE, "..", "..", "target", "release", "examples", "sweep")
MINUTES = float(sys.argv[1]) if len(sys.argv) > 1 else 90
GAMES = int(sys.argv[2]) if len(sys.argv) > 2 else 4000
FIELD = sys.argv[3] if len(sys.argv) > 3 else "house"
MARGIN = 0.5

best = {}
if os.path.exists(os.path.join(HERE, "best.txt")):
    for kv in open(os.path.join(HERE, "best.txt")).read().strip().split(","):
        k, v = kv.split("=")
        best[k] = v

# name: (kind, step) - kind "mul" multiplies, "add" adds, "int" adds integers
SPACE = {
    "offers_per_turn": ("int", 2), "money_offers_per_turn": ("int", 1), "min_offer_gain": ("add", 0.3),
    "accept_gain": ("add", 0.4), "accept_money_gain": ("add", 0.5), "their_gain_weight": ("add", 0.5),
    "their_gain_cap": ("mul", 1.5), "coin_short": ("mul", 1.4), "coin_rich": ("mul", 1.6),
    "coin_margin": ("mul", 1.5), "fill_per_round": ("add", 0.2), "spare_weight": ("mul", 1.3),
    "side_weight": ("mul", 1.25), "open_gain": ("add", 0.25), "joker_premium": ("int", 3),
    "final_bid_mult": ("mul", 1.5), "money_refusals": ("int", 1),
}
DEFAULTS = {"offers_per_turn": 6, "money_offers_per_turn": 2, "min_offer_gain": 0.6, "accept_gain": 1.0,
            "accept_money_gain": 2.0, "their_gain_weight": 1.0, "their_gain_cap": 2.0, "coin_short": 0.5,
            "coin_rich": 0.08, "coin_margin": 12.0, "fill_per_round": 0.4, "spare_weight": 0.5,
            "side_weight": 1.0, "open_gain": 0.5, "joker_premium": 8, "final_bid_mult": 3.0, "money_refusals": 2}


def val(k):
    return float(best.get(k, DEFAULTS.get(k, 0)))


def step(k, up):
    kind, s = SPACE[k]
    v = val(k)
    if kind == "mul":
        n = v * s if up else v / s
        if k == "dice_medal_weight":
            n = v / 0.3 if up else v * 0.3
        return round(n, 4)
    if kind == "int":
        return max(0, int(v + (s if up else -s)))
    return round(max(0.0, v + (s if up else -s)), 4)


def spec(d):
    return "t1{" + ",".join(f"{k}={v}" for k, v in d.items()) + "}"


def run(variants, seed):
    out = subprocess.run([EXE, "--games", str(GAMES), "--sizes", "4", "--field", FIELD, "--seed", str(seed),
                          "--variants", ";".join(variants)], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        m = re.match(r"\s*([\d.]+)%\s+([\d.]+)%\s+([\d.]+)\s+([\d.]+)", line)
        if m:
            rows.append((float(m.group(1)), float(m.group(3)), float(m.group(4))))
    return rows


log = open(os.path.join(HERE, "descend.log"), "a")
start = time.time()
seed = 20000 + random.randrange(10000) * 10
keys = list(SPACE)
while time.time() - start < MINUTES * 60:
    random.shuffle(keys)
    for i in range(0, len(keys), 6):
        if time.time() - start >= MINUTES * 60:
            break
        group = keys[i:i + 6]
        variants = [spec(best)]
        labels = ["current"]
        for k in group:
            for up in (True, False):
                v = step(k, up)
                if v == val(k):
                    continue
                d = dict(best)
                d[k] = v
                variants.append(spec(d))
                labels.append(f"{k}={v}")
        seed += GAMES
        rows = run(variants, seed)
        if len(rows) != len(variants):
            log.write(f"run failed for {labels}\n"); log.flush()
            continue
        cur = rows[0]
        ranked = sorted(range(1, len(rows)), key=lambda j: -rows[j][0])
        j = ranked[0]
        log.write(f"[{time.strftime('%H:%M')}] seed {seed} current {cur[0]:.2f}% place {cur[1]:.3f} | " +
                  " ".join(f"{labels[x]}:{rows[x][0]:.2f}" for x in ranked) + "\n")
        if rows[j][0] >= cur[0] + MARGIN and rows[j][1] >= cur[1]:
            k, v = labels[j].split("=")
            best[k] = v
            log.write(f"  ACCEPT {k}={v} ({rows[j][0]:.2f}% vs {cur[0]:.2f}%)\n")
            open(os.path.join(HERE, "best.txt"), "w").write(",".join(f"{a}={b}" for a, b in best.items()))
        log.flush()
log.write("done\n")
log.flush()
