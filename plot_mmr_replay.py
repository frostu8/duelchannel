import csv
import sys
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any, Self

import dacite
import matplotlib.pyplot as plt


def to_bool(input: Any):
    if isinstance(input, bool):
        return input
    elif isinstance(input, str):
        if input.casefold() == "YES".casefold():
            return True
        elif input.casefold() == "NO".casefold():
            return False
        else:
            raise ValueError(f"unexpected value {input}")
    else:
        raise TypeError(f"unexpected type {type(input)}")

@dataclass
class User:
    id: int
    short_id: str
    name: str
    games: int
    wlr: float
    rating: float
    deviation: float
    ordinal: float
    provisional: bool
    medal: bool
    new_medal: bool

    @classmethod
    def from_dict(cls, src: Mapping[str, Any]) -> Self:
        d = dict(src)

        if d["provisional"] == "PROV":
            d["provisional"] = True
        elif d["provisional"] == "VISIBLE":
            d["provisional"] = False

        d["medal"] = to_bool(d["medal"])
        d["new_medal"] = to_bool(d["new_medal"])

        return dacite.from_dict(cls, d, dacite.Config(type_hooks={int: int, float: float}))

    @property
    def color(self) -> str:
        if self.medal:
            return "gold"
        elif self.new_medal:
            return "orange"
        else:
            return "blue"

# Open the stdio and parse csv
reader = csv.DictReader(sys.stdin, escapechar='\\')

users = []
for row in reader:
    user = User.from_dict(row)

    # Skip players with too few games on record
    if user.provisional:
        continue

    users.append(user)

# Plot data
colors = [u.color for u in users]

fig,ax = plt.subplots(dpi=192)
plot = plt.scatter(x=[u.ordinal for u in users],
                   y=[u.wlr for u in users],
                   c=colors,
                   s=15)

plt.title("Player ratings")
plt.xlabel("DR")
plt.ylabel("Win/Loss Ratio")

# Create the annotation object
annotation = ax.annotate("", xy=(0,0), xytext=(-60,20),
                         textcoords="offset points",
                         size="large",
                         bbox={"boxstyle": "round", "fc": "w"},
                         arrowprops={"arrowstyle": "->"})
annotation.set_visible(False)

def update_annot(ind: dict[Any, Any]):
    pos = plot.get_offsets()[ind["ind"][0]]
    annotation.xy = pos
    text = "{}, {}".format(" ".join(list(map(str,ind["ind"]))), 
                           " ".join([users[n].name for n in ind["ind"]]))
    annotation.set_text(text)

    bbox = annotation.get_bbox_patch()
    if bbox is not None:
        bbox.set_facecolor(colors[ind["ind"][0]])
        bbox.set_alpha(0.4)
    

def hover(event):
    vis = annotation.get_visible()
    if event.inaxes == ax:
        cont, ind = plot.contains(event)
        if cont:
            update_annot(ind)
            annotation.set_visible(True)
            fig.canvas.draw_idle()
        else:
            if vis:
                annotation.set_visible(False)
                fig.canvas.draw_idle()

fig.canvas.mpl_connect("motion_notify_event", hover)

plt.show()
