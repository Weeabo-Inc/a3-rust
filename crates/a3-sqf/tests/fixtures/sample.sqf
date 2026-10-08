/*
    Hand-written sample exercising common SQF constructs (not from the game).
    Returns a hash map summarising a list of fake contacts.
*/
#line 1 "\tests\sample.sqf"
params [["_contacts", [], [[]]], ["_maxRange", 1000, [0]], ["_label", "summary", [""]]];

private _summary = createHashMap;
private _bySide = createHashMapFromArray [["west", []], ["east", []], ["other", []]];
scopeName "main";

if (_contacts isEqualTo []) exitWith {
    _summary set ["empty", true];
    _summary
};

{
    _x params ["_name", "_side", "_dist", ["_tags", []]];
    if (_dist > _maxRange) then { continue };

    private _bucket = switch (toLower _side) do {
        case "west": { "west" };
        case "east": { "east" };
        default { "other" };
    };
    (_bySide get _bucket) pushBack [_name, _dist];

    if ("hvt" in _tags) then {
        _summary set ["hvt", _name];
        if (_dist < 50) then { breakOut "main" };
    };
} forEach _contacts;

private _closest = [];
{
    private _list = _y;
    _list sort true;
    if (count _list > 0) then {
        _closest pushBack [_x, (_list select 0) select 0];
    };
} forEach _bySide;

private _total = 0;
for "_i" from 0 to (count _contacts - 1) do {
    _total = _total + ((_contacts select _i) select 2);
};

private _avg = if (count _contacts > 0) then { _total / count _contacts } else { 0 };
_summary set ["average", round (_avg * 100) / 100];
_summary set ["closest", _closest];
_summary set ["label", format ["%1 (%2 contacts)", _label, count _contacts]];

try {
    if (_avg < 0) then { throw "negative" };
} catch {
    diag_log format ["[sample] %1", _exception];
};

_summary
