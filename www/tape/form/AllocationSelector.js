Ext.define('PBS.TapeManagement.AllocationStore', {
    extend: 'Ext.data.Store',
    alias: 'store.allocationCalendarEventStore',

    field: ['value', 'text'],
    data: [
	{ value: 'continue', text: gettext('Continue') },
	{ value: 'always', text: gettext('Always') },
	{ value: '*:0/30', text: Ext.String.format(gettext("Every {0} minutes"), 30) },
	{ value: 'hourly', text: gettext("Every hour") },
	{ value: '0/2:00', text: gettext("Every two hours") },
	{ value: '2,22:30', text: gettext("Every day") + " 02:30, 22:30" },
	{ value: 'daily', text: gettext("Every day") + " 00:00" },
	{ value: 'mon..fri', text: gettext("Monday to Friday") + " 00:00" },
	{ value: 'mon..fri *:00', text: gettext("Monday to Friday") + ', ' + gettext("hourly") },
	{ value: 'sat 18:15', text: gettext("Every Saturday") + " 18:15" },
	{ value: 'monthly', text: gettext("Every first day of the Month") + " 00:00" },
	{ value: 'sat *-1..7 02:00', text: gettext("Every first Saturday of the month") + " 02:00" },
	{ value: 'yearly', text: gettext("First day of the year") + " 00:00" },
    ],
});

Ext.define('PBS.TapeManagement.AllocationSelector', {
    extend: 'PBS.form.CalendarEvent',
    alias: 'widget.pbsAllocationSelector',

    store: {
	type: 'allocationCalendarEventStore',
    },
});

