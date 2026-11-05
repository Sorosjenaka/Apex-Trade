import unittest
from tc3_metrics import START, VAL, END, metrics, segment


def trade(signal, entry, exit_, pnl=1.0, kind="TakeProfit"):
    return {"signal_ms": signal, "entry_ms": entry, "exit_ms": exit_, "exit_kind": kind, "pnl_net": pnl, "r": pnl}


class MetricsTests(unittest.TestCase):
    def test_boundary_and_forced_exit(self):
        trades = [trade(VAL-3, VAL-2, VAL-1), trade(VAL-1, VAL, VAL+1), trade(VAL, VAL+1, VAL+2), trade(END-3, END-2, END-1, kind="EndOfData")]
        self.assertEqual(len(segment(trades, START, VAL)), 1)
        self.assertEqual(len(segment(trades, VAL, END)), 1)

    def test_drawdown_and_empty_profit_factor(self):
        trades = [trade(1, 2, 3, 2), trade(4, 5, 6, -1), trade(7, 8, 9, -2)]
        self.assertEqual(metrics(trades)["max_dd_r"], 3)
        self.assertEqual(metrics(trades)["profit_factor"], 2/3)
        self.assertIsNone(metrics([trades[0]])["profit_factor"])
        self.assertIsNone(metrics([])["expectancy_r"])


if __name__ == "__main__":
    unittest.main()
