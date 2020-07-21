Ext.define('PBS.data.CalendarEventExamples', {
    extend: 'Ext.data.Store',
    alias: 'store.calendarEventExamples',

    field: ['value', 'text'],
    data: [
	//FIXME { value: '*/30', text: Ext.String.format(gettext("Every {0} minutes"), 30) },
	{ value: 'hourly', text: gettext("Every hour") },
	//FIXME { value: '*/2:00', text: gettext("Every two hours") },
	{ value: '2,22:30', text: gettext("Every day") + " 02:30, 22:30" },
	{ value: 'daily', text: gettext("Every day") + " 00:00" },
	{ value: 'mon..fri', text: gettext("Monday to Friday") + " 00:00" },
	//FIXME{ value: 'mon..fri */1:00', text: gettext("Monday to Friday") + ': ' + gettext("hourly") },
	{ value: 'sat 18:15', text: gettext("Every Saturday") + " 18:15" },
	//FIXME{ value: 'monthly', text: gettext("Every 1st of Month") + " 00:00" }, // not yet possible..
    ],
});

Ext.define('PBS.form.CalendarEvent', {
    extend: 'Ext.form.field.ComboBox',
    xtype: 'pbsCalendarEvent',

    editable: true,

    valueField: 'value',
    displayField: 'text',
    queryMode: 'local',

    config: {
	deleteEmpty: true,
    },
    // overide framework function to implement deleteEmpty behaviour
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
