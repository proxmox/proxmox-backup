Ext.define('PBS.Dashboard', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsDashboard',

    controller: {
	xclass: 'Ext.app.ViewController',

	openDashboardOptions: function() {
	    var me = this;
	    var viewModel = me.getViewModel();
	    Ext.create('Ext.window.Window', {
		modal: true,
		width: 300,
		title: gettext('Dashboard Options'),
		layout: {
		    type: 'auto'
		},
		items: [{
		    xtype: 'form',
		    bodyPadding: '10 10 10 10',
		    defaultButton: 'savebutton',
		    items: [{
			xtype: 'proxmoxintegerfield',
			itemId: 'hours',
			labelWidth: 100,
			anchor: '100%',
			allowBlank: false,
			minValue: 1,
			maxValue: 24,
			value: viewModel.get('hours'),
			fieldLabel: gettext('Hours to show')
		    }],
		    buttons: [{
			text: gettext('Save'),
			reference: 'loginButton',
			formBind: true,
			handler: function() {
			    var win = this.up('window');
			    var hours = win.down('#hours').getValue();
			    me.setHours(hours, true);
			    win.close();
			}
		    }]
		}]
	    }).show();
	},

	setHours: function(hours, setState) {
	    var me = this;
	    var viewModel = me.getViewModel();
	    viewModel.set('hours', hours);
	    viewModel.notify();

	    if (setState) {
		var sp = Ext.state.Manager.getProvider();
		sp.set('dashboard-hours', hours);
	    }
	},


	updateSubscription: function(store, records, success) {
	    if (!success) {
		return;
	    }
	    var me = this;
	    var viewmodel = me.getViewModel();

	    var subStatus = 2; // 2 = all good, 1 = different leves, 0 = none
	    var subLevel = "";
	},

	updateUsageStats: function(store, records, success) {
	    if (!success) {
		return;
	    }
	    if (records === undefined || records.length < 1) {
		return;
	    }
	    let me = this;
	    let viewmodel = me.getViewModel();

	    let res = records[0].data;

	    let cpu = res.cpu,
	        mem = res.memory,
	        root = res.root;

	    var cpuPanel = me.lookup('cpu');
	    cpuPanel.updateValue(cpu);

	    var memPanel = me.lookup('mem');
	    memPanel.updateValue(mem.used / mem.total);

	    var hdPanel = me.lookup('root');
	    hdPanel.updateValue(root.used / root.total);
	},

	updateTasks: function(store, records, success) {
	    if (!success) return;
	    let me = this;

	    records.sort((a, b) => a.data.duration - b.data.duration);
	    let top10 = records.slice(-10);
	    me.lookup('longesttasks').updateTasks(top10);

	    let data = {
		backup: {  error: 0, warning: 0, ok: 0, },
		prune: { error: 0, warning: 0, ok: 0, },
		garbage_collection: { error: 0, warning: 0, ok: 0, },
		sync: {  error: 0, warning: 0, ok: 0, },
	    };

	    records.forEach(record => {
		let type = record.data.worker_type;
		if (type === 'syncjob') {
		    type = 'sync';
		}

		if (data[type] && record.data.status) {
		    let parsed = Proxmox.Utils.parse_task_status(record.data.status);
		    data[type][parsed]++;
		}
	    });

	    me.lookup('tasksummary').updateTasks(data);
	},

	init: function(view) {
	    var me = this;
	    var sp = Ext.state.Manager.getProvider();
	    var hours = sp.get('dashboard-hours') || 12;
	    me.setHours(hours, false);
	}
    },

    viewModel: {
	data: {
	    timespan: 300, // in seconds
	    hours: 12, // in hours
	    error_shown: false,
	    'bytes_in': 0,
	    'bytes_out': 0,
	    'avg_ptime': 0.0
	},

	stores: {
	    usage: {
		storeid: 'dash-usage',
		type: 'update',
		interval: 3000,
		autoStart: true,
		autoLoad: true,
		autoDestroy: true,
		proxy: {
		    type: 'proxmox',
		    url: '/api2/json/nodes/localhost/status'
		},
		listeners: {
		    load: 'updateUsageStats'
		}
	    },
	    subscription: {
		storeid: 'dash-subscription',
		type: 'update',
		interval: 10000,
		autoStart: true,
		autoLoad: true,
		autoDestroy: true,
		proxy: {
		    type: 'proxmox',
		    url: '/api2/json/subscription'
		},
		listeners: {
		    load: 'updateSubscription'
		}
	    },
	    tasks: {
		storeid: 'dash-tasks',
		type: 'update',
		interval: 15000,
		autoStart: true,
		autoLoad: true,
		autoDestroy: true,
		model: 'proxmox-tasks',
		proxy: {
		    type: 'proxmox',
		    url: '/api2/json/status/tasks'
		},
		listeners: {
		    load: 'updateTasks'
		}
	    },
	}
    },

    title: gettext('Dashboard') + ' - WIP',

    layout: {
	type: 'column'
    },

    bodyPadding: '20 0 0 20',

    defaults: {
	columnWidth: 0.49,
	xtype: 'panel',
	margin: '0 20 20 0'
    },

    scrollable: true,

    items: [
	{
	    height: 250,
	    iconCls: 'fa fa-tasks',
	    title: gettext('Server Resources'),
	    bodyPadding: '0 20 0 20',
	    layout: {
		type: 'hbox',
		align: 'center'
	    },
	    defaults: {
		xtype: 'proxmoxGauge',
		spriteFontSize: '20px',
		flex: 1
	    },
	    items: [
		{
		    title: gettext('CPU'),
		    reference: 'cpu'
		},
		{
		    title: gettext('Memory'),
		    reference: 'mem'
		},
		{
		    title: gettext('Root Disk'),
		    reference: 'root'
		}
	    ]
	},
	{
	    xtype: 'pbsDatastoresStatistics',
	    height: 250,
	},
	{
	    xtype: 'pbsLongestTasks',
	    reference: 'longesttasks',
	    height: 250,
	},
	{
	    xtype: 'pbsRunningTasks',
	    height: 250,
	},
	{
	    xtype: 'pbsTaskSummary',
	    reference: 'tasksummary',
	},
	{
	    iconCls: 'fa fa-ticket',
	    title: 'Subscription',
	    height: 166,
	    reference: 'subscription',
	    xtype: 'pmgSubscriptionInfo',
	},
    ]
});

Ext.define('PMG.dashboard.SubscriptionInfo', {
    extend: 'Ext.panel.Panel',
    xtype: 'pmgSubscriptionInfo',

    data: {
	icon: 'question-circle',
	message: gettext('Unknown')
    },

    style: {
	cursor: 'pointer'
    },

    setSubStatus: function(status) {
	var me = this;
	var data = {};

	switch (status) {
	    case 2:
		data.icon = 'check green';
		data.message = gettext('Your subscription status is valid.');
		break;
	    case 1:
		data.icon = 'exclamation-triangle yellow';
		data.message = gettext('Warning: Your subscription levels are not the same.');
		break;
	    case 0:
		data.icon = 'times-circle red';
		data.message = gettext('You have at least one node without subscription.');
		break;
	    default:
		throw 'invalid subscription status';
	}
	me.update(data);
    },
    tpl: [
	'<table style="height: 100%;" class="dash">',
	'<tr><td class="center">',
	'<i class="fa fa-3x fa-{icon}"></i>',
	'</td><td class="center">{message}</td></tr>',
	'</table>'
    ],

    listeners: {
	click: {
	    element: 'body',
	    fn: function() {
		var mainview = this.component.up('mainview');
		mainview.getController().redirectTo('pmgSubscription');
	    }
	}
    }
});
