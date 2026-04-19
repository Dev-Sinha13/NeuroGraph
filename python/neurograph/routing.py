from __future__ import annotations

from dataclasses import dataclass


@dataclass(slots=True)
class RoutingConfig:
    escalation_confidence_threshold: float = 0.70
    escalation_node_count_threshold: int = 30
    privacy_lock: bool = True


def validate_routing_config(
    routing_config: RoutingConfig, confidence_config: dict
) -> list[str]:
    warnings: list[str] = []
    heuristic_ceiling = confidence_config["heuristic_range"]["max"]
    if routing_config.escalation_confidence_threshold > heuristic_ceiling:
        warnings.append(
            "RoutingConfig.escalation_confidence_threshold is above the heuristic_range ceiling. "
            "A dynamic codebase will escalate nearly every heuristic edge."
        )
    if not confidence_config["is_calibrated"]:
        warnings.append(
            "ConfidenceConfig is still uncalibrated, so routing thresholds are running against defaults."
        )
    return warnings

