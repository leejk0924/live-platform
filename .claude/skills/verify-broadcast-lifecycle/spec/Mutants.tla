------------------------------ MODULE Mutants ------------------------------
(***************************************************************************)
(* Broken variants of the code, one per property. Each spec/mutants/*.cfg  *)
(* swaps one action for its variant here with `<-` and checks the one      *)
(* property that must catch it. scripts/mutants.sh expects every one of    *)
(* them to FAIL; a mutant that passes means that property can no longer    *)
(* fail and proves nothing.                                                *)
(***************************************************************************)
EXTENDS BroadcastLifecycle

\* redeem without `b.status = ANY($5)`: admits a publisher in any status.
AuthAnyStatus(s) ==
  /\ sess[s] = "idle" /\ \A t \in 1..(s - 1) : sess[t] # "idle"
  /\ sess' = [sess EXCEPT ![s] = IF ~deleted THEN "authorized" ELSE "closed"]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, av, lv, avKick, stopPc,
                 stopKick, reaperVars>>

\* BroadcastEvent::Resume with from: [Interrupted, Ended].
ArrivedResumeFromEnded(s) ==
  LET match == status \in {"INTERRUPTED", "ENDED"} /\ ~deleted IN
  /\ av[s] = "sent" /\ NotBlocked(match)
  /\ IF match
     THEN SetStatus("LIVE") /\ av' = [av EXCEPT ![s] = "none"]
     ELSE av' = [av EXCEPT ![s] = "check"]
          /\ UNCHANGED <<status, liveOld, intOld, rTouched>>
  /\ UNCHANGED <<deleted, sess, lv, avKick, stopPc, stopKick, rPc, rHolds,
                 rStatus, rDeleted, rLiveOld, rIntOld, rOnAir, rKick, rKickSet,
                 failures>>

\* soft_delete without `status = ANY($3)`.
DeleteAnyStatus ==
  /\ NotBlocked(~deleted) /\ ~deleted
  /\ deleted' = TRUE
  /\ UNCHANGED <<status, liveOld, intOld, pubVars, stopPc, stopKick,
                 reaperVars>>

\* Reaper::tick expires before it reconciles: the pass goes probe -> expire
\* and the reconcile resume runs only after it.
RProbeOkExpireFirst ==
  /\ rPc = "probe"
  /\ rPc' = "expire" /\ rOnAir' = OnAirNow
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rKick, rKickSet>>

\* Reaper::tick without not_active: only expired broadcasts are kicked.
RNotActiveDropped ==
  /\ rPc = "notActive"
  /\ rPc' = "expire"
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick, rKickSet>>

\* reconcile without its second UPDATE (LIVE, not on air -> INTERRUPTED).
RInterruptDropped ==
  /\ rPc = "interrupt"
  /\ rPc' = "notActive"
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick, rKickSet>>

\* Reaper::tick without expire.
RExpireDropped ==
  /\ rPc = "expire"
  /\ rPc' = "demo"
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick, rKickSet>>
=============================================================================
