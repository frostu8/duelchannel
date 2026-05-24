import sys
import json
from dacite import Config
from enum import unique, Enum
from datetime import datetime
from typing import Self, TypeVar, Type
from dataclasses import dataclass, asdict
from openskill.models import BradleyTerryFull, BradleyTerryFullRating
import dacite

model = BradleyTerryFull()

@unique
class BattleStatus(Enum):
    ONGOING = 0
    CONCLUDED = 1
    CANCELLED = 2

@dataclass
class Rating:
    """
    A Duel Channel rating.
    """

    user_id: int
    rating: float
    deviation: float
    ordinal: float

    @classmethod
    def frommodel(cls, rating: BradleyTerryFullRating) -> Self:
        if rating.name is None:
            raise ValueError("no name given for player")
        else:
            id = int(rating.name)

        ordinal = rating.ordinal(alpha=200/rating.sigma,target=1500)
        return cls(id, rating.mu, rating.sigma, ordinal)

    def tomodel(self) -> BradleyTerryFullRating:
        return model.create_rating([self.rating, self.deviation], str(self.user_id))

@dataclass
class RatingRecord:
    """
    A Duel Channel rating record.
    """

    user_id: int
    period_id: int
    rating: float
    deviation: float
    inserted_at: datetime
    ordinal: float

    def tomodel(self) -> BradleyTerryFullRating:
        return model.create_rating([self.rating, self.deviation], str(self.user_id))

@dataclass
class Matchup:
    """
    A Duel Channel matchup.
    """

    opponent: RatingRecord
    status: BattleStatus
    position: int
    no_contest: bool
    finish_time: int

@dataclass
class InitialRating:
    rating: float
    deviation: float

@dataclass
class ModelConfig:
    """
    Model configuration.
    """

    period: str
    tau: float
    defaults: InitialRating

T = TypeVar("T")

def from_dict(ty: Type[T], data: dict) -> T:
    config = Config(cast=[BattleStatus], type_hooks={datetime: datetime.fromisoformat})
    return dacite.from_dict(ty, data, config)

# Start loop
# Listen for requests
def run():
    for line in sys.stdin:
        line = line.strip()

        # Parse request
        data = json.loads(line)
        name = data["type"]
        match name:
            case "UpdateConfig":
                config = from_dict(ModelConfig, data["config"])

                # Update new details
                model.tau = config.tau

                model.mu = config.defaults.rating
                model.sigma = config.defaults.deviation

                resp = {
                    "type": "UpdateConfig",
                }
            case "CreateRating":
                id = data["user_id"]

                # Make a rating in the model
                rating = model.rating(name=str(id))

                resp = {
                    "type": "CreateRating",
                    "rating": asdict(Rating.frommodel(rating)),
                }
            case "Rate":
                rating = from_dict(RatingRecord, data["rating"])
                matchups = [from_dict(Matchup, d) for d in data["matchups"]]

                # Create rating in model
                new_rating = rating.tomodel()

                # Assess new rating
                for matchup in matchups:
                    opponent_rating = matchup.opponent.tomodel()
                    opponent_position = 3 - matchup.position

                    [[new_rating], _] = model.rate(
                        [[new_rating], [opponent_rating]],
                        [matchup.position, opponent_position],
                        limit_sigma=True,
                    )

                # Return result
                resp = {
                    "type": "Rate",
                    "new_rating": asdict(Rating.frommodel(new_rating)),
                }
            case _:
                raise ValueError(f"unexpected event {name}")

        sys.stdout.write(f"{json.dumps(resp)}\n")
        sys.stdout.flush()

try:
    run()
except KeyboardInterrupt:
    pass
