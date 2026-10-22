// Tunable values of the clean-room engine. Every default was chosen by feel for this
// engine; the table drives both the rider and the host's mod menu.
#pragma once
#include "../include/sk8.h"

namespace sk8 {

enum Param : int
{
	// Pop & Jump
	PopLow, PopHigh, CrouchCharge, NolliePop, GrindPop,
	// Gravity & Speed
	Gravity, PushKick, PushPeriod, PushTop, PushLead, PushMongo, PopWait, RollDrag, AirDrag, Brake, BrakeLead, BrakeOut, Carve, Grip, SlideDrag, SlideSoft, GroundSnap,
	SwitchWait, SwitchTime,
	// Spin
	SpinRate, SpinEase, AirLevel,
	// Flips
	FlipBase, FlipPerRev, AirFlick, FlipLenience,
	// Bails
	LandClean, LandMax, SideMax, LandSkid, WallBail, ImpactBail, BailTime, BailSlide, Rise,
	// Grinds
	GrindCatch, GrindDrag, GrindBalance, GrindKeep, GrindOffKeep,
	// Manuals
	ManualWobble, ManualCorrect, ManualHold, ManualDrag,
	// Scoring
	ScoreScale, BankWait, GrindRate, ManualRate,
	ParamCount
};

const sk8_param *param_table();

struct Params
{
	float v[ParamCount];
	Params();
	float operator[](Param p) const { return v[p]; }
};

} // namespace sk8
