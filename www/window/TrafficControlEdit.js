Ext.define('PBS.window.TrafficControlEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsTrafficControlEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'sysadmin_traffic_control',
    width: 800,
    height: 600,

    isAdd: true,

    subject: gettext('Traffic Control Rule'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/traffic-control';
	let name = initialConfig.name;

	me.isCreate = !name;
	me.url = name ? `${baseurl}/${name}` : baseurl;
	me.method = name ? 'PUT' : 'POST';
	return { };
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	weekdays: ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'],

	dowChanged: function(field, value) {
	    let me = this;
	    let record = field.getWidgetRecord();
	    if (record === undefined) {
		// this is sometimes called before a record/column is initialized
		return;
	    }
	    let col = field.getWidgetColumn();
	    record.set(col.dataIndex, value);
	    record.commit();

	    me.updateTimeframeField();
	},

	timeChanged: function(field, value) {
	    let me = this;
	    if (value === null) {
		return;
	    }
	    let record = field.getWidgetRecord();
	    if (record === undefined) {
		// this is sometimes called before a record/column is initialized
		return;
	    }
	    let col = field.getWidgetColumn();
	    let hours = value.getHours().toString().padStart(2, '0');
	    let minutes = value.getMinutes().toString().padStart(2, '0');
	    record.set(col.dataIndex, `${hours}:${minutes}`);
	    record.commit();

	    me.updateTimeframeField();
	},

	addTimeframe: function() {
	    let me = this;
	    me.lookup('timeframes').getStore().add({
		start: "00:00",
		end: "23:59",
		mon: true,
		tue: true,
		wed: true,
		thu: true,
		fri: true,
		sat: true,
		sun: true,
	    });

	    me.updateTimeframeField();
	},

	updateTimeframeField: function() {
	    let me = this;

	    let timeframes = [];
	    me.lookup('timeframes').getStore().each((rec) => {
		let timeframe = '';
		let days = me.weekdays.filter(day => rec.data[day]);
		if (days.length < 7 && days.length > 0) {
		    timeframe += days.join(',') + ' ';
		}
		let { start, end } = rec.data;

		timeframe += `${start}-${end}`;
		timeframes.push(timeframe);
	    });

	    let field = me.lookup('timeframe');
	    field.suspendEvent('change');
	    field.setValue(timeframes.join(';'));
	    field.resumeEvent('change');
	},

	removeTimeFrame: function(field) {
	    let me = this;
	    let record = field.getWidgetRecord();
	    if (record === undefined) {
		// this is sometimes called before a record/column is initialized
		return;
	    }

	    me.lookup('timeframes').getStore().remove(record);
	    me.updateTimeframeField();
	},

	parseTimeframe: function(timeframe) {
	    let me = this;
	    let [, days, start, end] = /^(?:(\S*)\s+)?([0-9:]+)-([0-9:]+)$/.exec(timeframe) || [];

	    if (start === '0') {
		start = "00:00";
	    }

	    let record = {
		start,
		end,
	    };

	    if (!days) {
		days = 'mon..sun';
	    }

	    days = days.split(',');
	    days.forEach((day) => {
		if (record[day]) {
		    return;
		}

		if (me.weekdays.indexOf(day) !== -1) {
		    record[day] = true;
		} else {
		    // we have a range 'xxx..yyy'
		    let [startDay, endDay] = day.split('..');
		    let startIdx = me.weekdays.indexOf(startDay);
		    let endIdx = me.weekdays.indexOf(endDay);

		    if (endIdx < startIdx) {
			endIdx += me.weekdays.length;
		    }

		    for (let dayIdx = startIdx; dayIdx <= endIdx; dayIdx++) {
			let curDay = me.weekdays[dayIdx%me.weekdays.length];
			if (!record[curDay]) {
			    record[curDay] = true;
			}
		    }
		}
	    });

	    return record;
	},

	setGridData: function(field, value) {
	    let me = this;
	    if (!value) {
		return;
	    }

	    value = value.split(';');
	    let records = value.map((timeframe) => me.parseTimeframe(timeframe));
	    me.lookup('timeframes').getStore().setData(records);
	},

	control: {
	    'grid checkbox': {
		change: 'dowChanged',
	    },
	    'grid timefield': {
		change: 'timeChanged',
	    },
	    'grid button': {
		click: 'removeTimeFrame',
	    },
	    'field[name=timeframe]': {
		change: 'setGridData',
	    },
	},
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let me = this;
	    let isCreate = me.up('window').isCreate;

	    if (!values.network) {
		values.network = ['0.0.0.0/0', '::/0'];
	    } else {
		values.network = [...new Set(values.network.split(/\s*,\s*/))];
	    }

	    if ('timeframe' in values && !values.timeframe) {
		delete values.timeframe;
	    }
	    if (values.timeframe && !Ext.isArray(values.timeframe)) {
		values.timeframe = [...new Set(values.timeframe.split(';'))];
	    }

	    if (!isCreate) {
		PBS.Utils.delete_if_default(values, 'timeframe');
		PBS.Utils.delete_if_default(values, 'rate-in');
		PBS.Utils.delete_if_default(values, 'rate-out');
		PBS.Utils.delete_if_default(values, 'burst-in');
		PBS.Utils.delete_if_default(values, 'burst-out');
		if (typeof values.delete === 'string') {
		    values.delete = values.delete.split(',');
		}
	    }

	    return values;
	},
	column1: [
	    {
		xtype: 'pmxDisplayEditField',
		name: 'name',
		fieldLabel: gettext('Name'),
		renderer: Ext.htmlEncode,
		allowBlank: false,
		minLength: 3,
		cbind: {
		    editable: '{isCreate}',
		},
	    },
	    {
		xtype: 'pmxBandwidthField',
		name: 'rate-in',
		fieldLabel: gettext('Rate In'),
		emptyText: gettext('Unlimited'),
		submitAutoScaledSizeUnit: true,
	    },
	    {
		xtype: 'pmxBandwidthField',
		name: 'rate-out',
		fieldLabel: gettext('Rate Out'),
		emptyText: gettext('Unlimited'),
		submitAutoScaledSizeUnit: true,
	    },
	],

	column2: [
	    {
		xtype: 'proxmoxtextfield',
		name: 'comment',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
		fieldLabel: gettext('Comment'),
	    },
	    {
		xtype: 'pmxBandwidthField',
		name: 'burst-in',
		fieldLabel: gettext('Burst In'),
		emptyText: gettext('Same as Rate'),
		submitAutoScaledSizeUnit: true,
	    },
	    {
		xtype: 'pmxBandwidthField',
		name: 'burst-out',
		fieldLabel: gettext('Burst Out'),
		emptyText: gettext('Same as Rate'),
		submitAutoScaledSizeUnit: true,
	    },
	],

	columnB: [
	    {
		xtype: 'proxmoxtextfield',
		fieldLabel: gettext('Network(s)'),
		name: 'network',
		emptyText: `0.0.0.0/0, ::/0 (${gettext('Apply on all Networks')})`,
		autoEl: {
		    tag: 'div',
		    'data-qtip': gettext('A comma-separated list of networks to apply the (shared) limit.'),
		},
	    },
	    {
		xtype: 'displayfield',
		fieldLabel: gettext('Timeframes'),
	    },
	    {
		xtype: 'fieldcontainer',
		items: [
		    {
			xtype: 'grid',
			height: 300,
			scrollable: true,
			reference: 'timeframes',
			viewConfig: {
			    emptyText: gettext('Apply Always'),
			},
			store: {
			    fields: ['start', 'end', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'],
			    data: [],
			},
			columns: [
			    {
				text: gettext('Time Start'),
				xtype: 'widgetcolumn',
				dataIndex: 'start',
				widget: {
				    xtype: 'timefield',
				    isFormField: false,
				    format: 'H:i',
				    formatText: 'HH:MM',
				},
				flex: 1,
			    },
			    {
				text: gettext('Time End'),
				xtype: 'widgetcolumn',
				dataIndex: 'end',
				widget: {
				    xtype: 'timefield',
				    isFormField: false,
				    format: 'H:i',
				    formatText: 'HH:MM',
				    maxValue: '23:59',
				},
				flex: 1,
			    },
			    {
				text: gettext('Mon'),
				xtype: 'widgetcolumn',
				dataIndex: 'mon',
				width: 60,
				widget: {
				    xtype: 'checkbox',
				    isFormField: false,
				},
			    },
			    {
				text: gettext('Tue'),
				xtype: 'widgetcolumn',
				dataIndex: 'tue',
				width: 60,
				widget: {
				    xtype: 'checkbox',
				    isFormField: false,
				},
			    },
			    {
				text: gettext('Wed'),
				xtype: 'widgetcolumn',
				dataIndex: 'wed',
				width: 60,
				widget: {
				    xtype: 'checkbox',
				    isFormField: false,
				},
			    },
			    {
				text: gettext('Thu'),
				xtype: 'widgetcolumn',
				dataIndex: 'thu',
				width: 60,
				widget: {
				    xtype: 'checkbox',
				    isFormField: false,
				},
			    },
			    {
				text: gettext('Fri'),
				xtype: 'widgetcolumn',
				dataIndex: 'fri',
				width: 60,
				widget: {
				    xtype: 'checkbox',
				    isFormField: false,
				},
			    },
			    {
				text: gettext('Sat'),
				xtype: 'widgetcolumn',
				dataIndex: 'sat',
				width: 60,
				widget: {
				    xtype: 'checkbox',
				    isFormField: false,
				},
			    },
			    {
				text: gettext('Sun'),
				xtype: 'widgetcolumn',
				dataIndex: 'sun',
				width: 60,
				widget: {
				    xtype: 'checkbox',
				    isFormField: false,
				},
			    },
			    {
				xtype: 'widgetcolumn',
				width: 40,
				widget: {
				    xtype: 'button',
				    iconCls: 'fa fa-trash-o',
				},
			    },
			],
		    },
		],
	    },
	    {
		xtype: 'button',
		text: gettext('Add'),
		iconCls: 'fa fa-plus-circle',
		handler: 'addTimeframe',
	    },
	    {
		xtype: 'hidden',
		reference: 'timeframe',
		name: 'timeframe',
	    },
	],
    },

    doSetValues: function(data) {
	let me = this;

	// NOTE: it can make sense to have any-ip (::/0 and 0/0) and specific ones in the same set
	// so only check for "is default" when there really just two networks
	if (data.network?.length === 2) {
	    let nets = [...new Set(data.network)]; // only the set of unique networks
	    if (nets.find(net => net === '0.0.0.0/0') && nets.find(net => net === '::/0')) {
		delete data.network;
	    }
	}
	if (data.network?.length) {
	    data.network = data.network.join(', ');
	}

	if (Ext.isArray(data.timeframe)) {
	    data.timeframe = data.timeframe.join(';');
	}

	me.setValues(data);
    },

    initComponent: function() {
	let me = this;

	me.callParent();

	if (!me.isCreate) {
	    me.load({
		success: ({ result: { data } }) => me.doSetValues(data),
	    });
	}
    },
});
