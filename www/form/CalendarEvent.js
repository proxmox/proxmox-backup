Ext.define('PBS.data.CalendarEventExamples', {
    extend: 'Ext.data.Store',
    alias: 'store.calendarEventExamples',

    field: ['value', 'text'],
    data: [
	{ value: '*:0/30', text: Ext.String.format(gettext("Every {0} minutes"), 30) },
	{ value: 'hourly', text: gettext("Every hour") },
	{ value: '0/2:00', text: gettext("Every two hours") },
	{ value: '2,22:30', text: gettext("Every day") + " 02:30, 22:30" },
	{ value: '21:00', text: gettext("Every day") + " 21:00" },
	{ value: 'daily', text: gettext("Every day") + " 00:00" },
	{ value: 'mon..fri 00:00', text: gettext("Monday to Friday") + " 00:00" },
	{ value: 'mon..fri *:00', text: gettext("Monday to Friday") + ', ' + gettext("hourly") },
	{ value: 'sat 18:15', text: gettext("Every Saturday") + " 18:15" },
	{ value: 'monthly', text: gettext("Every first day of the Month") + " 00:00" },
	{ value: 'sat *-1..7 02:00', text: gettext("Every first Saturday of the month") + " 02:00" },
	{ value: 'yearly', text: gettext("First day of the year") + " 00:00" },
    ],
});

Ext.define('PBS.form.CalendarEvent', {
    extend: 'Ext.form.field.ComboBox',
    xtype: 'pbsCalendarEvent',

    editable: true,

    valueField: 'value',
    queryMode: 'local',

    matchFieldWidth: false,

    config: {
	deleteEmpty: true,
    },
    // override framework function to implement deleteEmpty behaviour
    getSubmitData: function() {
	let me = this, data = null;
	if (!me.disabled && me.submitValue) {
	    let val = me.getSubmitValue();
	    if (val !== null && val !== '' && val !== '__default__') {
		data = {};
		data[me.getName()] = val;
	    } else if (me.getDeleteEmpty()) {
		data = {};
		data.delete = me.getName();
	    }
	}
	return data;
    },

    triggers: {
	clear: {
	    cls: 'pmx-clear-trigger',
	    weight: -1,
	    hidden: true,
	    handler: function() {
		this.triggers.clear.setVisible(false);
		this.setValue('');
	    },
	},
    },

    listeners: {
	change: function(field, value) {
	    let canClear = (value ?? '') !== '';
	    field.triggers.clear.setVisible(canClear);
	},
    },

    store: {
	type: 'calendarEventExamples',
    },

    tpl: [
	'<ul class="x-list-plain"><tpl for=".">',
	    '<li role="option" class="x-boundlist-item">{text}</li>',
	'</tpl></ul>',
    ],

    displayTpl: [
	'<tpl for=".">',
	'{value}',
	'</tpl>',
    ],
});
