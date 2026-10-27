#!/usr/bin/env python3
"""A failover proof needs coherent fresh membership/term/leader observations."""
import importlib.util
import copy
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("failover", Path(__file__).resolve().parents[1] / "run-management-failover-test.py")
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


def snapshots():
    return {name: {"schema": "charlotte.cluster.v2", "fresh_committed": True, "truncated": False,
                   "served_by_leader": name == "a", "raft": {"term": 4, "leader": "a", "served_by": name},
                   "nodes": [{"node_key": node, "member": True} for node in ("a", "b", "c")]}
            for name in ("a", "b", "c")}


class FailoverObservations(unittest.TestCase):
    def projections(self):
        rows = [{"vip": "10.0.2.42", "port": port, "protocol": 6, "backend_name": None,
                 "projection_present": True, "member_count": 3, "eligible_nodes": ["a", "b"],
                 "advertiser_node": "a", "epoch": 7} for port in (443, 7444)]
        return {name: {"ingress": copy.deepcopy(rows)} for name in ("a", "b")}

    def test_complete_survivor_projection_agrees_without_removing_the_dead_voter(self):
        self.assertIsNotNone(fixture.ingress_projection(self.projections(), {"a", "b"}))
        self.assertIsNone(fixture.ingress_projection(self.projections(), {"a"}))

    def test_unavailable_partial_duplicate_and_dead_backend_projections_reject(self):
        for replacement in ([], [None], self.projections()["a"]["ingress"][:1]):
            values = self.projections(); values["b"]["ingress"] = replacement
            self.assertIsNone(fixture.ingress_projection(values, {"a", "b"}))
        for field, value in (("projection_present", False), ("member_count", 2),
                             ("eligible_nodes", ["a", "b", "dead"]), ("eligible_nodes", ["b", "a"]),
                             ("epoch", 8), ("advertiser_node", "dead"), ("backend_name", "other")):
            values = self.projections(); values["b"]["ingress"][0][field] = value
            with self.subTest(field=field, value=value):
                self.assertIsNone(fixture.ingress_projection(values, {"a", "b"}))
        values = self.projections(); values["b"]["ingress"][1] = values["b"]["ingress"][0]
        self.assertIsNone(fixture.ingress_projection(values, {"a", "b"}))
    def test_initial_and_replacement_leader_require_coherent_term(self):
        initial = snapshots()
        self.assertEqual(fixture.topology(initial, 3), ("a", "a", 4))
        with_dead = {name: value for name, value in initial.items() if name != "a"}
        for name, value in with_dead.items():
            value["raft"].update(leader="b", term=5)
            value["served_by_leader"] = name == "b"
        self.assertEqual(fixture.topology(with_dead, 3, "a", 4), ("b", "b", 5))
        self.assertIsNone(fixture.topology(with_dead, 3, "a", 5))
        self.assertIsNone(fixture.topology(with_dead, 3, "b", 4))

    def test_stale_truncated_unavailable_and_disagreeing_views_reject(self):
        for field, value in (("fresh_committed", False), ("truncated", True), ("schema", "retired")):
            changed = snapshots()
            changed["b"][field] = value
            self.assertIsNone(fixture.topology(changed, 3))
        for value in (None, {}, [], "unavailable"):
            changed = snapshots()
            changed["b"] = value
            self.assertIsNone(fixture.topology(changed, 3))
        changed = snapshots()
        changed["b"]["raft"]["term"] = 5
        self.assertIsNone(fixture.topology(changed, 3))
        changed = snapshots()
        changed["b"]["raft"]["leader"] = "b"
        self.assertIsNone(fixture.topology(changed, 3))

    def test_equal_member_counts_cannot_hide_different_membership(self):
        changed = snapshots()
        changed["b"]["nodes"][0]["node_key"] = "other"
        self.assertIsNone(fixture.topology(changed, 3))
        changed = snapshots()
        changed["b"]["nodes"][0]["member"] = False
        self.assertIsNone(fixture.topology(changed, 3))

    def test_exactly_one_observed_owner_must_report_that_leader(self):
        changed = snapshots()
        changed["b"]["served_by_leader"] = True
        self.assertIsNone(fixture.topology(changed, 3))
        changed = snapshots()
        changed["a"]["served_by_leader"] = False
        self.assertIsNone(fixture.topology(changed, 3))
        changed = snapshots()
        changed["a"]["raft"]["served_by"] = "other"
        self.assertIsNone(fixture.topology(changed, 3))


if __name__ == "__main__":
    unittest.main()
