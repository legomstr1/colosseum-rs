"""Coordinate descent over tuned-2 params using the sweep harness, against a
strong mixed field that includes tuned-1 and live-input, at 3-5 players.
`partner_gain` (offers never leave the partner worse off) and
`their_gain_weight` (partners' gain counts fully when ranking offers) are
fixed, so trades stay mutually beneficial.

Each batch tries one step up and one down for a group of params against the
current best on the same seeds; the best variant wins if it beats the current
one by `MARGIN` win-rate points and doesn't place worse. Writes progress to
descend.log and the current best to best.txt.

python tuning/tuned-2/descend.py [minutes] [games]
SWEEP_EXE overrides the harness binary (so a rebuild mid-run is safe).
"""
import subprocess, sys, time, re, os, random

HERE = os.path.dirname(os.path.abspath(__file__))
EXE = os.environ.get("SWEEP_EXE") or os.path.join(HERE, "..", "..", "target", "release", "examples", "sweep")
MINUTES = float(sys.argv[1]) if len(sys.argv) > 1 else 50
GAMES = int(sys.argv[2]) if len(sys.argv) > 2 else 3000
FIELD = "house|oracle|maestro|chameleon|t1{}|li{}"
MARGIN = 0.5
GROUP = 3

best = {}
if os.path.exists(os.path.join(HERE, "best.txt")):
    txt = open(os.path.join(HERE, "best.txt")).read().strip()
    for kv in filter(None, txt.split(",")):
        k, v = kv.split("=")
        best[k] = v

PLANS = ["LTEE", "TLEE", "LETE", "LTEP", "LEET", "TEEL", "LTPE"]
# name: (kind, step, default) - "mul" multiplies, "add" adds, "int" adds
# integers, "choice" picks another option from the list.
SPACE = {
    "top_events": ("int", 2, 8), "commit_round": ("int", 1, 2), "uncommitted_weight": ("add", 0.25, 0.5),
    "side_weight": ("mul", 1.25, 1.25), "spare_weight": ("mul", 1.3, 0.5), "fill_per_round": ("add", 0.15, 0.4),
    "rival_take": ("add", 0.2, 0.4), "plan": ("choice", PLANS, "LTEE"), "mid_buy_gain": ("choice", [0, 3, 6, 10], 0),
    "open_gain": ("add", 0.25, 0.5), "final_open_gain": ("add", 1.0, 3.0), "joker_premium": ("int", 3, 8),
    "final_bid_mult": ("mul", 1.5, 3.0), "big_batch_gain": ("add", 2.0, 8.0), "big_batch_premium": ("int", 2, 2),
    "offers_per_turn": ("int", 3, 10), "money_offers_per_turn": ("int", 1, 3), "min_offer_gain": ("add", 0.3, 0.6),
    "accept_gain": ("add", 0.4, 1.0), "accept_money_gain": ("add", 0.5, 2.0), "coin_short": ("mul", 1.4, 0.5),
    "coin_early_short": ("mul", 1.5, 0.15), "short_from_round": ("int", 1, 4), "coin_rich": ("mul", 1.5, 0.128),
    "coin_margin": ("mul", 1.5, 12.0), "coin_final": ("mul", 3.0, 0.03), "ticket_slack": ("add", 0.3, 1.0),
    "podium_weight": ("add", 3.0, 0.0), "dice_medal_weight": ("mul", 0.3, 100.0),
}


def val(k):
    kind, _, d = SPACE[k]
    v = best.get(k, d)
    return v if kind == "choice" else float(v)


def step(k, up):
    kind, s, _ = SPACE[k]
    v = val(k)
    if kind == "choice":
        others = [o for o in s if str(o) != str(v)]
        return random.choice(others)
    if kind == "mul":
        return round(v * s if up else v / s, 4)
    if kind == "int":
        return max(0, int(v + (s if up else -s)))
    return round(max(0.0, v + (s if up else -s)), 4)


def spec(d):
    return "t2{" + ",".join(f"{k}={v}" for k, v in d.items()) + "}"


def run(variants, seed):
    out = subprocess.run([EXE, "--games", str(GAMES), "--sizes", "3,4,5", "--field", FIELD, "--seed", str(seed),
                          "--variants", ";".join(variants)], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        m = re.match(r"\s*([\d.]+)%\s+([\d.]+)%\s+([\d.]+)\s+([\d.]+)", line)
        if m:
            rows.append((float(m.group(1)), float(m.group(3)), float(m.group(4))))
    return rows


log = open(os.path.join(HERE, "descend.log"), "a")
start = time.time()
seed = 30000 + random.randrange(10000) * 10
keys = list(SPACE)
while time.time() - start < MINUTES * 60:
    random.shuffle(keys)
    for i in range(0, len(keys), GROUP):
        if time.time() - start >= MINUTES * 60:
            break
        group = keys[i:i + GROUP]
        variants = [spec(best)]
        labels = ["current"]
        for k in group:
            for up in (True, False):
                v = step(k, up)
                if str(v) == str(val(k)) or f"{k}={v}" in labels:
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
