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
		    type: 'auto',
		},
		items: [{
		    xtype: 'form',
		    bodyPadding: '10 10 10 10',
		    defaultButton: 'savebutton',
		    items: [{
			xtype: 'proxmoxintegerfield',
			itemId: 'days',
			labelWidth: 100,
			anchor: '100%',
			allowBlank: false,
			minValue: 1,
			maxValue: 60,
			value: viewModel.get('days'),
			fieldLabel: gettext('Days to show'),
		    }],
		    buttons: [{
			text: gettext('Save'),
			reference: 'savebutton',
			formBind: true,
			handler: function() {
			    var win = this.up('window');
			    var days = win.down('#days').getValue();
			    me.setDays(days, true);
			    win.close();
			},
		    }],
		}],
	    }).show();
	},

	setDays: function(days, setState) {
	    var me = this;
	    var viewModel = me.getViewModel();
	    viewModel.set('days', days);
	    viewModel.notify();

	    viewModel.getStore('tasks').reload();

	    if (setState) {
		var sp = Ext.state.Manager.getProvider();
		sp.set('dashboard-days', days);
	    }
	},

	updateRepositoryStatus: function(store, records, success) {
	    if (!success) { return; }
	    let me = this;
	    let view = me.getView();
	    view.down('#repositoryStatus').setRepositoryInfo(records[0].data['standard-repos']);
	},

	updateSubscription: function(store, records, success) {
	    if (!success) { return; }
	    let me = this;
	    let view = me.getView();
	    let status = records[0].data.status || 'unknown';
	    // 2 = all good, 1 = different levels, 0 = none
	    let subscriptionActive = status.toLowerCase() === 'active';
	    let subStatus = status.toLowerCase() === 'active' ? 2 : 0;
	    me.lookup('subscription').setSubStatus(subStatus);
	    view.down('#repositoryStatus').setSubscriptionStatus(subscriptionActive);
	},

	updateTasks: function(store, records, success) {
	    if (!success) return;
	    let me = this;
	    let viewModel = me.getViewModel();

	    records.sort((a, b) => a.data.duration - b.data.duration);
	    let top10 = records.slice(-10);
	    me.lookup('longesttasks').updateTasks(top10);

	    let data = {
		backup: { error: 0, warning: 0, ok: 0 },
		prune: { error: 0, warning: 0, ok: 0 },
		garbage_collection: { error: 0, warning: 0, ok: 0 },
		sync: { error: 0, warning: 0, ok: 0 },
		verify: { error: 0, warning: 0, ok: 0 },
		'tape-backup': { error: 0, warning: 0, ok: 0 },
		'tape-restore': { error: 0, warning: 0, ok: 0 },
	    };

	    records.forEach(record => {
		let task = record.data;
		let type = task.worker_type;
		if (type === 'syncjob') {
		    type = 'sync';
		}

		if (type.startsWith('verif')) {
		    type = 'verify';
		}

		if (type.startsWith('prune')) {
		    type = 'prune';
		}

		if (type.startsWith('tape-backup')) {
		    type = 'tape-backup';
		}

		if (data[type] && task.status) {
		    let parsed = Proxmox.Utils.parse_task_status(task.status);
		    data[type][parsed]++;
		}
	    });

	    me.lookup('tasksummary').updateTasks(data, viewModel.get('sinceEpoch'));
	},

	init: function(view) {
	    var me = this;
	    var sp = Ext.state.Manager.getProvider();
	    var days = sp.get('dashboard-days') || 30;
	    me.setDays(days, false);

	    view.mon(sp, 'statechange', function(provider, key, value) {
		if (key !== 'summarycolumns') {
		    return;
		}
		Proxmox.Utils.updateColumns(view);
	    });
	},
    },

    viewModel: {
	data: {
	    days: 30,
	},

	formulas: {
	    sinceEpoch: (get) => (Date.now()/1000 - get('days') * 24*3600).toFixed(0),
	},

	stores: {
	    repositories: {
		storeid: 'dash-repositories',
		type: 'update',
		interval: 15000,
		autoStart: true,
		autoLoad: true,
		autoDestroy: true,
		proxy: {
		    type: 'proxmox',
		    url: '/api2/json/nodes/localhost/apt/repositories',
		},
		listeners: {
		    load: 'updateRepositoryStatus',
		},
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
		    url: '/api2/json/nodes/localhost/subscription',
		},
		listeners: {
		    load: 'updateSubscription',
		},
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
		    url: '/api2/json/nodes/localhost/tasks',
		    extraParams: {
			limit: 0,
			since: '{sinceEpoch}',
		    },
		},
		listeners: {
		    load: 'updateTasks',
		},
	    },
	},
    },

    listeners: {
	resize: function(panel) {
	    Proxmox.Utils.updateColumns(panel);
	},
    },

    title: gettext('Dashboard'),

    layout: {
	type: 'column',
    },

    bodyPadding: '20 0 0 20',

    minWidth: 700,

    defaults: {
	columnWidth: 0.49,
	xtype: 'panel',
	margin: '0 20 20 0',
    },

    tools: [
	{
	    type: 'gear',
	    tooltip: gettext('Edit dashboard settings'),
	    handler: 'openDashboardOptions',
	},
    ],

    scrollable: true,

    items: [
	{
	    xtype: 'pbsNodeInfoPanel',
	    reference: 'nodeInfo',
	    height: 290,
	},
	{
	    xtype: 'pbsDatastoresStatistics',
	    height: 290,
	},
	{
	    xtype: 'pbsLongestTasks',
	    bind: {
		title: gettext('Longest Tasks') + ' (' +
		Ext.String.format(gettext('{0} days'), '{days}') + ')',
	    },
	    reference: 'longesttasks',
	    height: 290,
	},
	{
	    xtype: 'pbsRunningTasks',
	    height: 290,
	},
	{
	    bind: {
		title: gettext('Task Summary') + ' (' +
		Ext.String.format(gettext('{0} days'), '{days}') + ')',
	    },
	    xtype: 'pbsTaskSummary',
	    height: 250,
	    reference: 'tasksummary',
	},
	{
	    iconCls: 'fa fa-ticket',
	    title: 'Subscription',
	    height: 250,
	    reference: 'subscription',
	    xtype: 'pbsSubscriptionInfo',
	},
    ],
});

Ext.define('PBS.dashboard.SubscriptionInfo', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsSubscriptionInfo',

    style: {
	cursor: 'pointer',
    },

    layout: {
	type: 'hbox',
	align: 'middle',
    },

    items: [
	{
	    xtype: 'box',
	    itemId: 'icon',
	    data: {
		icon: 'question-circle',
	    },
	    width: 100,
	    tpl: '<center><i class="fa fa-3x fa-{icon}"></i></center>',
	},
	{
	    flex: 1,
	    xtype: 'box',
	    data: {
		message: gettext('Unknown'),
	    },
	    itemId: 'message',
	    tpl: '<center>{message}</center>',
	},
    ],

    setSubStatus: function(status) {
	var me = this;
	let icon = '';
	let message = '';

	switch (status) {
	    case 2:
		icon = 'check good';
		message = gettext('Your subscription status is valid.');
		break;
	    case 1:
		icon = 'exclamation-triangle warning';
		message = gettext('Warning: Your subscription levels are not the same.');
		break;
	    case 0:
		icon = 'times-circle critical';
		message = `<h1>${gettext('No valid subscription')}</h1>${PBS.Utils.noSubKeyHtml}`;
		break;
	    default:
		throw 'invalid subscription status';
	}
	me.getComponent('icon').update({ icon });
	me.getComponent('message').update({ message });
    },

    listeners: {
	click: {
	    element: 'body',
	    fn: function() {
		var mainview = this.component.up('mainview');
		mainview.getController().redirectTo('pbsSubscription');
	    },
	},
    },
});
